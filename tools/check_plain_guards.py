"""In the plain game (W462: the game on its own Direct3D 8, with no MGE XE), MGE XE's MWScript instructions and its
macro functions still run, so each one must do nothing there behind `if (Session::plain())`. This check of an MGE XE
tree fails, with the function's name, on a body that breaks one of these rules:

- each `execute()` of `d3d8/cpp/mwse/*.cpp` has the guard, with exactly the condition `Session::plain()`; it pops its
  arguments before the guard (the plain branch must pop them too, or the script stack breaks), so no pop comes after
  the guard, and no action comes before it (a push, a return value, MGE XE's HUD, distant land, post shaders, a set of
  the bridge);
- the plain branch ends in a return statement, so the rest of the body does not run in the plain game; a branch with
  no braces is the return statement itself;
- the plain branch of an `execute()` pushes a result when the rest of the body pushes one (the count is not checked: a
  loop of pushes counts as one push statement);
- each `MacroFunctions::` body of `d3d8/cpp/mge/macrofunctions.cpp` starts with the guard, and its branch returns;
- every `::execute(` and every `MacroFunctions::` name of the files is a body that the check parsed, so no header of
  another form escapes it.

    python -I tools/check_plain_guards.py --mge DIR    # an MGE XE tree
    python -I tools/check_plain_guards.py --self-test

Comments are not code. The exit code is 1 on a finding, and when a file or a body is not found."""
import re
import sys
from pathlib import Path

GUARD = "Session::plain()"
GUARD_IF = re.compile(r"if\s*\(\s*Session::plain\(\)\s*\)")
EXECUTE = re.compile(r"\b(\w+)::execute\s*\([^)]*\)\s*\{")
EXECUTE_NAME = re.compile(r"::execute\s*\(")
MACRO = re.compile(r"\bMacroFunctions::(\w+)\s*\([^)]*\)\s*\{")
MACRO_NAME = re.compile(r"\bMacroFunctions::\w+")
POP = re.compile(r"vmPop\w*\(")
PUSH = re.compile(r"vmPush\w*\(")
ACTION = re.compile(r"vmPush\w*\(|vmSetReturn|MGEhud::|DistantLand::|PostShaders::|MWBridge::get\(\)->set")
COMMENT = re.compile(r"//[^\n]*|/\*.*?\*/", re.DOTALL)


def block_end(text, start):
    """The index after the brace that closes the brace at `start`, or None."""
    depth = 0
    for at in range(start, len(text)):
        if text[at] == "{":
            depth += 1
        elif text[at] == "}":
            depth -= 1
            if depth == 0:
                return at + 1
    return None


def bodies(text, header):
    """(name, body with its braces) of each function of `header` in a text without comments."""
    for match in header.finditer(text):
        start = match.end() - 1
        end = block_end(text, start)
        if end is not None:
            yield match.group(1), text[start:end]


def plain_branch(body, guard):
    """(the branch of the guard match `guard`, the rest of the body after it). A branch without braces ends at its `;`."""
    rest = body[guard.end():].lstrip()
    start = len(body) - len(rest)
    end = block_end(body, start) if rest.startswith("{") else body.index(";", start) + 1
    return body[start:end], body[end:]


def returns(branch):
    """True when the last statement of the branch is a return statement."""
    inner = branch[1:-1] if branch.startswith("{") else branch
    inner = inner.rstrip()
    if not inner.endswith(";"):
        return False
    statement = inner[:-1]
    statement = statement[max(statement.rfind(";"), statement.rfind("{"), statement.rfind("}")) + 1:].strip()
    return statement == "return" or statement.startswith("return ") or statement.startswith("return(")


def guard_problems(name, body):
    """(the guard match or None, the findings of the guard and its branch) of a body."""
    guard = GUARD_IF.search(body)
    if guard is None:
        if GUARD in body:
            return None, ["%s: the condition of the guard is not exactly `if (%s)`" % (name, GUARD)]
        return None, ["%s: no `if (%s)` guard" % (name, GUARD)]
    before = body[:guard.start()]
    if before.count("{") - before.count("}") != 1:
        return guard, ["%s: the guard is inside another block, so the body can run past it in the plain game" % name]
    branch, _ = plain_branch(body, guard)
    if not returns(branch):
        return guard, ["%s: the plain branch does not end in a return, so the rest of the body runs" % name]
    return guard, []


def execute_problems(name, body):
    """The findings of one execute() body."""
    guard, problems = guard_problems(name, body)
    if guard is None:
        return problems
    if POP.search(body, guard.end()):
        problems.append("%s: a pop after the guard: pop every argument before it" % name)
    if ACTION.search(body[:guard.start()]):
        problems.append("%s: an action before the guard" % name)
    branch, rest = plain_branch(body, guard)
    if PUSH.search(rest) and not PUSH.search(branch):
        problems.append("%s: the plain branch pushes no result, and the rest of the body does" % name)
    if PUSH.search(branch) and not PUSH.search(rest):
        problems.append("%s: the plain branch pushes a result, and the rest of the body does not" % name)
    return problems


def macro_problems(name, body):
    """The findings of one MacroFunctions body: its first statement is the guard, and its branch returns."""
    full = "MacroFunctions::" + name
    guard, problems = guard_problems(full, body)
    if guard is not None and body[1:guard.start()].strip():
        problems.append("%s: the first statement is not the `if (%s)` guard" % (full, GUARD))
    return problems


def check_text(text, header, names, judge):
    """(count of bodies, findings) of a file's text. Each match of `names` must be a parsed body."""
    code = COMMENT.sub("", text)
    found = list(bodies(code, header))
    problems = [problem for name, body in found for problem in judge(name, body)]
    named = len(names.findall(code))
    if named != len(found):
        problems.append("%d of %d named functions parsed as bodies: a header of another form" % (len(found), named))
    return len(found), problems


def check(tree):
    """(instructions, macros, findings) of an MGE XE tree."""
    problems = []
    instructions = 0
    for path in sorted((tree / "d3d8" / "cpp" / "mwse").glob("*.cpp")):
        count, found = check_text(path.read_text(encoding="utf-8", errors="replace"), EXECUTE, EXECUTE_NAME, execute_problems)
        instructions += count
        problems += ["%s: %s" % (path.name, problem) for problem in found]
    macro_file = tree / "d3d8" / "cpp" / "mge" / "macrofunctions.cpp"
    macros, found = check_text(macro_file.read_text(encoding="utf-8", errors="replace"), MACRO, MACRO_NAME, macro_problems)
    problems += ["macrofunctions.cpp: %s" % problem for problem in found]
    if not instructions or not macros:
        problems.append("no instruction or no macro found: the layout of the tree changed")
    return instructions, macros, problems


def execute_findings(text):
    return check_text(text, EXECUTE, EXECUTE_NAME, execute_problems)


def macro_findings(text):
    return check_text(text, MACRO, MACRO_NAME, macro_problems)


def self_test():
    good = ("bool A::execute(mwseInstruction* _this) {\n    VMLONG a;\n    if (!_this->vmPop(&a)) { return false; }\n"
            "    if (Session::plain()) {\n        _this->vmPush(VMREGTYPE(0));\n        return true;\n    }\n"
            "    return _this->vmPush(VMREGTYPE(MWBridge::get()->getX(a)));\n}\n")
    assert execute_findings(good) == (1, [])
    # A loop of pushes on the path and six pushes in the branch: no count, no finding (mwseModelBounds).
    loop = ("bool B::execute(mwseInstruction* _this) {\n    if (Session::plain()) {\n"
            + "        _this->vmPush(VMFLOAT(0));\n" * 6 + "        return true;\n    }\n"
            "    for (int i = 0; i < 6; i++) { _this->vmPush(VMFLOAT(f[i])); }\n    return true;\n}\n")
    assert execute_findings(loop) == (1, [])
    # A branch without braces that is the return, and a comment that names the guard is not the guard.
    bare = "bool C::execute(mwseInstruction* _this) {\n    if (Session::plain()) return true;\n    return true;\n}\n"
    assert execute_findings(bare) == (1, [])
    commented = "bool D::execute(mwseInstruction* _this) {\n    // Session::plain() is not needed here\n    return true;\n}\n"
    assert execute_findings(commented)[1] == ["D: no `if (Session::plain())` guard"]
    late_pop = good.replace("if (!_this->vmPop(&a)) { return false; }\n", "").replace("    return _this->vmPush", "    _this->vmPop(&a);\n    return _this->vmPush")
    assert execute_findings(late_pop)[1] == ["A: a pop after the guard: pop every argument before it"]
    early_action = good.replace("    if (Session::plain())", "    MGEhud::hide(a);\n    if (Session::plain())")
    assert execute_findings(early_action)[1] == ["A: an action before the guard"]
    no_result = good.replace("        _this->vmPush(VMREGTYPE(0));\n", "")
    assert execute_findings(no_result)[1] == ["A: the plain branch pushes no result, and the rest of the body does"]
    extra_result = loop.replace("    for (int i = 0; i < 6; i++) { _this->vmPush(VMFLOAT(f[i])); }\n", "")
    assert execute_findings(extra_result)[1] == ["B: the plain branch pushes a result, and the rest of the body does not"]
    # The Reviewer's plants of verdict 390. (1) A branch that pushes and does not return: the body goes on to the HUD.
    falls_through = ("bool P1::execute(mwseInstruction* _this) {\n    if (Session::plain()) {\n        _this->vmPush(VMREGTYPE(0));\n    }\n"
                     "    MGEhud::hide(0);\n    return _this->vmPush(VMREGTYPE(1));\n}\n")
    assert execute_findings(falls_through)[1] == ["P1: the plain branch does not end in a return, so the rest of the body runs"]
    bare_action = "bool P1b::execute(mwseInstruction* _this) {\n    if (Session::plain()) MGEhud::hide(0);\n    return true;\n}\n"
    assert execute_findings(bare_action)[1] == ["P1b: the plain branch does not end in a return, so the rest of the body runs"]
    # (2) A header of another spelling is parsed; one that the parse cannot take is counted.
    spaced = good.replace("bool A::execute(mwseInstruction* _this)", "bool A::execute(mwseInstruction *_this)")
    assert execute_findings(spaced) == (1, [])
    unparsed = good.replace("bool A::execute(mwseInstruction* _this) {", "bool A::execute(mwseInstruction* _this) const noexcept {")
    assert execute_findings(unparsed) == (0, ["0 of 1 named functions parsed as bodies: a header of another form"])
    # (3) A condition that is not exactly the guard.
    negated = good.replace("if (Session::plain())", "if (!Session::plain())")
    assert execute_findings(negated)[1] == ["A: the condition of the guard is not exactly `if (Session::plain())`"]
    # (5) A guard nested under another condition: with a <= 0 the HUD call runs in the plain game.
    nested_guard = ("bool P5::execute(mwseInstruction* _this) {\n    VMLONG a;\n    if (!_this->vmPop(&a)) { return false; }\n"
                    "    if (a > 0) {\n        if (Session::plain()) {\n            _this->vmPush(VMREGTYPE(0));\n            return true;\n        }\n    }\n"
                    "    MGEhud::hide(a);\n    return _this->vmPush(VMREGTYPE(1));\n}\n")
    assert execute_findings(nested_guard)[1] == ["P5: the guard is inside another block, so the body can run past it in the plain game"]
    # (6) A statement after the return of the branch.
    after_return = good.replace("        return true;\n    }\n", "        return true;\n        MGEhud::hide(a);\n    }\n")
    assert execute_findings(after_return)[1] == ["A: the plain branch does not end in a return, so the rest of the body runs"]
    # (4) A macro whose guard has an empty branch.
    macro = "void MacroFunctions::A() {\n    // MWSE can call it.\n    if (Session::plain()) {\n        return;\n    }\n    go();\n}\n"
    assert macro_findings(macro) == (1, [])
    empty = macro.replace("        return;\n", "")
    assert macro_findings(empty)[1] == ["MacroFunctions::A: the plain branch does not end in a return, so the rest of the body runs"]
    late_macro = macro.replace("    if (Session::plain())", "    go();\n    if (Session::plain())")
    assert macro_findings(late_macro)[1] == ["MacroFunctions::A: the first statement is not the `if (Session::plain())` guard"]
    assert macro_findings(macro + "static void f() { MacroFunctions::A(); }\n")[1] == ["1 of 2 named functions parsed as bodies: a header of another form"]
    # A return inside a nested block of the branch is not its last statement.
    nested = macro.replace("        return;\n", "        if (x) { return; }\n")
    assert macro_findings(nested)[1] == ["MacroFunctions::A: the plain branch does not end in a return, so the rest of the body runs"]
    assert mge_tree(["--mge", "master", "--mge", "tree"]) == Path("tree")
    print("check_plain_guards: self-test passed")


def mge_tree(argv):
    """The tree of the last --mge: checks.py adds the tree of a worktree's hook after its own."""
    if "--mge" not in argv:
        raise SystemExit("usage: check_plain_guards.py --mge DIR | --self-test")
    at = len(argv) - 1 - argv[::-1].index("--mge")
    if at + 1 >= len(argv):
        raise SystemExit("usage: check_plain_guards.py --mge DIR | --self-test")
    return Path(argv[at + 1])


def main(argv):
    if "--self-test" in argv:
        self_test()
        return 0
    tree = mge_tree(argv)
    if not (tree / "d3d8" / "cpp" / "mge" / "macrofunctions.cpp").is_file():
        print("plain guards: no MGE XE tree at %s" % tree)
        return 1
    instructions, macros, problems = check(tree)
    for problem in problems:
        print("plain guards: %s" % problem)
    if problems:
        return 1
    print("plain guards: %d instructions and %d macro functions keep the plain game's rules in %s" % (instructions, macros, tree))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
