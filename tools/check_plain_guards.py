"""In the plain game (W462: the game on its own Direct3D 8, with no MGE XE), MGE XE's MWScript instructions and its
macro functions still run, so each one must do nothing there behind `Session::plain()`. This check of an MGE XE tree
fails, with the function's name, on a body that breaks one of these rules:

- each `execute()` of `d3d8/cpp/mwse/*.cpp` has the guard; it pops its arguments before the guard (the plain branch
  must pop them too, or the script stack breaks), so no pop comes after the guard, and no action comes before it (a
  push, a return value, MGE XE's HUD, distant land, post shaders, a set of the bridge);
- the plain branch of an `execute()` pushes a result when the rest of the body pushes one (the count is not checked: a
  loop of pushes counts as one push statement);
- each `MacroFunctions::` body of `d3d8/cpp/mge/macrofunctions.cpp` starts with the guard.

    python -I tools/check_plain_guards.py --mge DIR    # an MGE XE tree
    python -I tools/check_plain_guards.py --self-test

Comments are not code. The exit code is 1 on a finding, and when a file or a body is not found."""
import re
import sys
from pathlib import Path

GUARD = "Session::plain()"
EXECUTE = re.compile(r"bool (\w+)::execute\(mwseInstruction\* _this\)\s*\{")
MACRO = re.compile(r"void MacroFunctions::(\w+)\(\)\s*\{")
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
    """(the branch of the guard at `guard`, the rest of the body after it). A branch without braces ends at its `;`."""
    after = body.index(")", guard + len(GUARD)) + 1
    rest = body[after:].lstrip()
    start = len(body) - len(rest)
    end = block_end(body, start) if rest.startswith("{") else body.index(";", start) + 1
    return body[start:end], body[end:]


def execute_problems(name, body):
    """The findings of one execute() body."""
    guard = body.find(GUARD)
    if guard < 0:
        return ["%s: no %s guard" % (name, GUARD)]
    problems = []
    if POP.search(body, guard):
        problems.append("%s: a pop after the guard: pop every argument before it" % name)
    if ACTION.search(body[:guard]):
        problems.append("%s: an action before the guard" % name)
    branch, rest = plain_branch(body, guard)
    if PUSH.search(rest) and not PUSH.search(branch):
        problems.append("%s: the plain branch pushes no result, and the rest of the body does" % name)
    if PUSH.search(branch) and not PUSH.search(rest):
        problems.append("%s: the plain branch pushes a result, and the rest of the body does not" % name)
    return problems


def macro_problems(name, body):
    """The finding of one MacroFunctions body: its first statement is not the guard."""
    if not body[1:].lstrip().startswith("if (%s)" % GUARD):
        return ["MacroFunctions::%s: the first statement is not the %s guard" % (name, GUARD)]
    return []


def check_text(text, header, judge):
    """(count of bodies, findings) of a file's text; a header that the parse missed is a finding too."""
    code = COMMENT.sub("", text)
    found = list(bodies(code, header))
    problems = [problem for name, body in found for problem in judge(name, body)]
    headers = len(header.findall(code))
    if headers != len(found):
        problems.append("%d of %d bodies parsed" % (len(found), headers))
    return len(found), problems


def check(tree):
    """(instructions, macros, findings) of an MGE XE tree."""
    problems = []
    instructions = 0
    for path in sorted((tree / "d3d8" / "cpp" / "mwse").glob("*.cpp")):
        count, found = check_text(path.read_text(encoding="utf-8", errors="replace"), EXECUTE, execute_problems)
        instructions += count
        problems += ["%s: %s" % (path.name, problem) for problem in found]
    macro_file = tree / "d3d8" / "cpp" / "mge" / "macrofunctions.cpp"
    macros, found = check_text(macro_file.read_text(encoding="utf-8", errors="replace"), MACRO, macro_problems)
    problems += ["macrofunctions.cpp: %s" % problem for problem in found]
    if not instructions or not macros:
        problems.append("no instruction or no macro found: the layout of the tree changed")
    return instructions, macros, problems


def self_test():
    good = ("bool A::execute(mwseInstruction* _this) {\n    VMLONG a;\n    if (!_this->vmPop(&a)) { return false; }\n"
            "    if (Session::plain()) {\n        _this->vmPush(VMREGTYPE(0));\n        return true;\n    }\n"
            "    return _this->vmPush(VMREGTYPE(MWBridge::get()->getX(a)));\n}\n")
    assert check_text(good, EXECUTE, execute_problems) == (1, [])
    # A loop of pushes on the path and six pushes in the branch: no count, no finding (mwseModelBounds).
    loop = ("bool B::execute(mwseInstruction* _this) {\n    if (Session::plain()) {\n"
            + "        _this->vmPush(VMFLOAT(0));\n" * 6 + "        return true;\n    }\n"
            "    for (int i = 0; i < 6; i++) { _this->vmPush(VMFLOAT(f[i])); }\n    return true;\n}\n")
    assert check_text(loop, EXECUTE, execute_problems) == (1, [])
    # A branch without braces, and a comment that names the guard is not the guard.
    bare = "bool C::execute(mwseInstruction* _this) {\n    if (Session::plain()) return true;\n    return true;\n}\n"
    assert check_text(bare, EXECUTE, execute_problems) == (1, [])
    commented = "bool D::execute(mwseInstruction* _this) {\n    // Session::plain() is not needed here\n    return true;\n}\n"
    assert check_text(commented, EXECUTE, execute_problems)[1] == ["D: no Session::plain() guard"]
    late_pop = good.replace("if (!_this->vmPop(&a)) { return false; }\n", "").replace("    return _this->vmPush", "    _this->vmPop(&a);\n    return _this->vmPush")
    assert check_text(late_pop, EXECUTE, execute_problems)[1] == ["A: a pop after the guard: pop every argument before it"]
    early_action = good.replace("    if (Session::plain())", "    MGEhud::hide(a);\n    if (Session::plain())")
    assert check_text(early_action, EXECUTE, execute_problems)[1] == ["A: an action before the guard"]
    no_result = good.replace("        _this->vmPush(VMREGTYPE(0));\n", "")
    assert check_text(no_result, EXECUTE, execute_problems)[1] == ["A: the plain branch pushes no result, and the rest of the body does"]
    extra_result = loop.replace("    for (int i = 0; i < 6; i++) { _this->vmPush(VMFLOAT(f[i])); }\n", "")
    assert check_text(extra_result, EXECUTE, execute_problems)[1] == ["B: the plain branch pushes a result, and the rest of the body does not"]
    macro = "void MacroFunctions::A() {\n    // MWSE can call it.\n    if (Session::plain()) {\n        return;\n    }\n    go();\n}\n"
    assert check_text(macro, MACRO, macro_problems) == (1, [])
    late_macro = macro.replace("    if (Session::plain())", "    go();\n    if (Session::plain())")
    assert check_text(late_macro, MACRO, macro_problems)[1] == ["MacroFunctions::A: the first statement is not the Session::plain() guard"]
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
