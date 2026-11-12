"""The checks of a Lua file that the game will run, before the game starts: its syntax, and a
local function that a line above its declaration uses.

    python -I tools/lua_checks.py FILE...     # the findings of each file; exit 1 with one
    python -I tools/lua_checks.py --self-test

The runner (run_own_device.py) runs both on its --shader-batch and its --lua-file, at a start and
with --check, and refuses the run on a finding. A syntax check passes the second fault: the use
reads the global of that name, which is nil, and the batch fails in the game (W381, the
Reviewer's verdict 334).

LuaJIT parses the file with `loadstring`, which runs nothing. Its place is the key "luajit" of
local.json, else the luajit.exe of the MWSE checkout beside the main checkout of this repository
(MWSE/MWSE/deps/luajit2/src). Without it, or when it does not run, the check is a finding: it
fails closed.

The second check is a port of Rob's use_before_local.py (W381). It reads only `local function
NAME`: a plain `local NAME` is often a parameter or a loop variable higher up (12 false alarms in
one batch). Before the search it blanks the comments (`--` and `--[[ ]]`) and the string literals
(quoted and long), so a name in them is no hit. A use as a field (`.NAME`, `:NAME`) is no hit. It
follows the blocks (`function`, `do`, `if` and `repeat` open one; `end` and `until` close it) and
the locals of each block (the `local` lists, the parameters, the `for` variables). A use is a hit
only when the block of the declaration is the use's block or encloses it, and no local of that
name is in scope at the use. A forward declaration (`local f` ... `f = function`) is not a
declaration for it, so it neither flags nor clears that form.

Its limits, each a false alarm on valid Lua:
- a global of the same name that the file defines above (`function helper() end`, `helper()`,
  `local function helper() end`);
- a local of a `repeat` body that its `until` condition reads: the block closes at `until`.
"""

import os
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO, local, main_checkout  # noqa: E402

LUAJIT_KEY = "luajit"
# The MWSE checkout of the place beside the main checkout: a worktree in C:/tmp has none beside it.
MWSE_LUAJIT = ("MWSE", "MWSE", "deps", "luajit2", "src", "luajit.exe")
LONG_OPEN = re.compile(r"\[(=*)\[")


def luajit():
    """The LuaJIT of this machine for the syntax check: the key "luajit" of local.json, else the
    one of the MWSE checkout beside the main checkout of this repository."""
    return local(LUAJIT_KEY, os.path.join(os.path.dirname(main_checkout() or REPO), *MWSE_LUAJIT))


def lua_string(text):
    """`text` as a Lua string literal."""
    return '"%s"' % text.replace("\\", "\\\\").replace('"', '\\"')


def syntax_problem(name, text, luajit_exe):
    """The fault of the syntax of the Lua `text` of the file `name` as LuaJIT gives it, or None.
    LuaJIT reads a copy of the text from a file in binary mode (in text mode a byte 0x1A ends the
    input) and parses it as `loadfile` would. It names the file by its base name: Lua cuts a longer
    name of a chunk to 60 characters, and the line number with it."""
    if not os.path.isfile(luajit_exe):
        return "no LuaJIT at %s for the syntax check: set the key %r of local.json" % (luajit_exe, LUAJIT_KEY)
    with tempfile.TemporaryDirectory() as temp:
        copy = os.path.join(temp, "chunk.lua")
        with open(copy, "wb") as file:
            file.write(text.encode("utf-8"))
        script = "local f = assert(io.open(%s, 'rb')) local s = f:read('*a') f:close() assert(loadstring(s, %s))" % (
            lua_string(copy), lua_string("=" + os.path.basename(name)))
        try:
            result = subprocess.run([luajit_exe, "-e", script], capture_output=True, text=True, encoding="utf-8",
                                    errors="replace")
        except OSError as error:
            return "LuaJIT at %s did not run (%s): set the key %r of local.json" % (luajit_exe, error, LUAJIT_KEY)
    if result.returncode == 0:
        return None
    lines = (result.stderr or result.stdout).strip().splitlines()
    first = lines[0] if lines else "it ended with %d" % result.returncode
    # LuaJIT names the -e chunk first: the file's own place and fault follow it. Without that name
    # the output is not LuaJIT's parse of the file.
    if "(command line):" not in first:
        return "LuaJIT at %s did not run (%s): set the key %r of local.json" % (luajit_exe, first, LUAJIT_KEY)
    return "the Lua syntax: %s" % re.sub(r"^.*?\(command line\):\d+: ", "", first)


def code_text(text):
    """`text` with its comments and string literals blanked: each of their characters is a space,
    except the line ends, so the lines keep their numbers."""
    out = []
    i, n = 0, len(text)

    def blank(end):
        out.append("".join(c if c == "\n" else " " for c in text[i:end]))
        return end

    while i < n:
        if text.startswith("--", i):
            long = LONG_OPEN.match(text, i + 2)
            if long:
                close = text.find("]%s]" % long.group(1), long.end())
                i = blank(n if close < 0 else close + len(long.group(1)) + 2)
            else:
                line_end = text.find("\n", i)
                i = blank(n if line_end < 0 else line_end)
        elif text[i] == "[" and LONG_OPEN.match(text, i):
            long = LONG_OPEN.match(text, i)
            close = text.find("]%s]" % long.group(1), long.end())
            i = blank(n if close < 0 else close + len(long.group(1)) + 2)
        elif text[i] in "\"'":
            quote, j = text[i], i + 1
            while j < n and text[j] != quote and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            i = blank(min(j + 1, n))
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


KEYWORDS = frozenset("and break do else elseif end false for function goto if in local nil not or repeat return then "
                     "true until while".split())
TOKEN = re.compile(r"[A-Za-z_]\w*|==|~=|<=|>=|\.\.\.?|::|\S")
# The words that open a block and those that close one. `while` and `for` open theirs at their `do`;
# the branches of an `if` share one block, which is enough for the names that a block holds.
OPENS = frozenset(("function", "do", "if", "repeat"))
CLOSES = frozenset(("end", "until"))


def tokens(code):
    """[(line, token)] of the Lua `code` (comments and strings blanked)."""
    found = []
    for number, line in enumerate(code.splitlines(), 1):
        found.extend((number, match.group(0)) for match in TOKEN.finditer(line))
    return found


def name_list(items, at):
    """The names of a list `a, b, c` from items[at] on, and the index after it."""
    names = []
    while at < len(items) and re.match(r"[A-Za-z_]\w*$", items[at][1]) and items[at][1] not in KEYWORDS:
        names.append(items[at][1])
        at += 1
        if at < len(items) and items[at][1] == ",":
            at += 1
        else:
            break
    return names, at


def uses_before_local(text):
    """[(name, the line of a use, the line of the declaration, the use's line)] for each read of a
    name above a `local function NAME` of the Lua `text`, where the block of the declaration is the
    block of the read or encloses it, and no local of that name is in scope at the read. A local of
    a block is a name of a `local` list, a parameter of a function, or a variable of a `for`; a name
    before a single `=` is a key or an assignment, not a read; a name after `.` or `:` is a field.
    A read above several such declarations gives one hit, with the nearest one below it."""
    raw = text.splitlines()
    items = tokens(code_text(text))
    # Each block has a number; `blocks` is the path of numbers from the file's block to this one.
    scopes = [set()]
    blocks = [0]
    next_block = 1
    pending = []
    declarations = []  # (name, line, the number of the block that holds it)
    reads = []  # (name, line, the block path at the read)
    i = 0
    while i < len(items):
        number, word = items[i]
        before = items[i - 1][1] if i else ""
        after = items[i + 1][1] if i + 1 < len(items) else ""
        opened = None
        if word == "local" and after == "function":
            # The name is a local of this block from here on; `function` opens its body next.
            if i + 2 < len(items):
                name = items[i + 2][1]
                scopes[-1].add(name)
                declarations.append((name, number, blocks[-1]))
            i += 1
        elif word == "local":
            names, i = name_list(items, i + 1)
            scopes[-1].update(names)
        elif word == "for":
            # The variables of a `for` are the locals of the block that its `do` opens.
            pending, i = name_list(items, i + 1)
        elif word == "function":
            # The name of `function a.b:c(...)` is not a read; the parameters are the body's locals.
            j = i + 1
            while j < len(items) and items[j][1] != "(":
                j += 1
            params, i = name_list(items, j + 1)
            opened = set(params)
        else:
            if word in OPENS:
                opened = set(pending) if word == "do" else set()
                pending = []
            elif word in CLOSES:
                if len(scopes) > 1:
                    scopes.pop()
                    blocks.pop()
            # A read: not a field (after `.` or `:`), not a key or a target (before a single `=`).
            elif (re.match(r"[A-Za-z_]", word) and word not in KEYWORDS and before not in (".", ":") and after != "="
                  and not any(word in scope for scope in scopes)):
                reads.append((word, number, tuple(blocks)))
            i += 1
        if opened is not None:
            scopes.append(opened)
            blocks.append(next_block)
            next_block += 1

    hits = []
    for name, line, path in reads:
        below = [at for declared, at, block in declarations if declared == name and at > line and block in path]
        if below:
            hits.append((name, line, min(below), raw[line - 1].strip()[:100]))
    return hits


def problems(name, text, luajit_exe):
    """The findings of the Lua `text` of the file `name`, as lines."""
    found = []
    syntax = syntax_problem(name, text, luajit_exe)
    if syntax:
        found.append(syntax)
    for function, used, declared, line in uses_before_local(text):
        found.append("the local function %s is used on line %d and declared on line %d (a use above its declaration "
                     "reads the global %s): %s" % (function, used, declared, function, line))
    return found


# A reduced copy of the batch of commit 12ab3fcb (docs/scripts/w381/batch-crowd.lua): glideTo
# calls setHover, which is declared below it. The syntax is good.
USE_ABOVE = """local function say(text) print(text) end

-- A glide from the player's real position.
local function glideTo(to, yaw, seconds)
    setHover(false)
    say("glide")
end

local hovering = false
local function setHover(on)
    hovering = on
end

glideTo({ 0, 0, 0 }, 90, 8)
"""


def self_test():
    # The use above a declaration: the reduced 12ab3fcb batch has one, on line 5 of line 10.
    assert uses_before_local(USE_ABOVE) == [("setHover", 5, 10, "setHover(false)")], uses_before_local(USE_ABOVE)
    # The clean form: the declaration first.
    clean = USE_ABOVE.replace("local hovering = false\nlocal function setHover(on)\n    hovering = on\nend\n\n", "")
    clean = "local hovering = false\nlocal function setHover(on)\n    hovering = on\nend\n\n" + clean
    assert uses_before_local(clean) == [], uses_before_local(clean)
    # The nulls of the port: the name in a comment, a long comment, a quoted or a long string, or
    # as a field above the declaration is no hit; the line numbers stay.
    for above in ('-- setHover(true)\n', '--[[ setHover(true)\nsetHover(false) ]]\n', 'say("setHover")\n', "say('it is setHover')\n",
                  'say([[setHover]])\n', 'say([==[\nsetHover ]] ]==])\n', 'tes3.setHover(1)\n', 'obj:setHover(1)\n',
                  'say("a \\" setHover")\n'):
        text = above + "local function setHover(on) end\n"
        assert uses_before_local(text) == [], (above, uses_before_local(text))
        assert len(code_text(text).splitlines()) == len(text.splitlines()), above
    # A real use after a blanked string on the same line is still a hit.
    assert uses_before_local('say("x") setHover(1)\nlocal function setHover() end\n') == [
        ("setHover", 1, 2, 'say("x") setHover(1)')]
    # The scope (the false alarms of batch-actor-arms.lua, W415): a key of a table, a target of an
    # assignment, and a local, a parameter or a `for` variable of the same name in scope are no hit.
    for above in ("local ARMS = { ports = true, other = 1 }\n",
                  "ports = 1\n",
                  "local function now()\n    local ports, b = 1, 2\n    return ports + b\nend\n",
                  "local function f(ports) return ports end\n",
                  "for ports, v in pairs({}) do print(ports) end\n",
                  "for ports = 1, 2 do print(ports) end\n",
                  "if true then\n    local ports = 1\n    print(ports)\nend\n",
                  "local t = function(a, ports) return ports end\n"):
        text = above + "local function ports() end\n"
        assert uses_before_local(text) == [], (above, uses_before_local(text))
    # A local whose block ended before the use does not cover it: the use reads the global.
    ended = ("local function a()\n    local ports = 1\n    return ports\nend\n"
             "local function b() return ports() end\n"
             "local function ports() end\n")
    assert uses_before_local(ended) == [("ports", 5, 6, "local function b() return ports() end")], uses_before_local(ended)
    # A comparison is a read, not an assignment.
    assert uses_before_local("print(ports == 1)\nlocal function ports() end\n") == [("ports", 1, 2, "print(ports == 1)")]
    # A forward declaration is not a declaration for the check: no hit, by design.
    assert uses_before_local("local setHover\nlocal function a() setHover() end\nsetHover = function() end\n") == []
    # The block of the declaration must hold the use (verdict 337). A declaration inside another
    # function does not reach a use outside it: a builtin of the same name above, and the inner
    # helper of a sibling function, are no hit.
    builtin = "print('a')\nlocal function g()\n    local function print() end\nend\n"
    assert uses_before_local(builtin) == [], uses_before_local(builtin)
    sibling = ("local function a() return helper end\n"
               "local function b()\n    local function helper() end\n    return helper\nend\n")
    assert uses_before_local(sibling) == [], uses_before_local(sibling)
    # A use above a second declaration of the name, in the second one's block, is a hit; the first
    # declaration's block ended before it.
    second = ("local function a()\n    local function h() end\n    h()\nend\n"
              "local function b()\n    h()\n    local function h() end\nend\n")
    assert uses_before_local(second) == [("h", 6, 7, "h()")], uses_before_local(second)
    # The null of the block rule: a use in a function below a declaration of the enclosing block is
    # still a hit (the 12ab3fcb form, one level deeper).
    deeper = "local function a()\n    if x then setHover() end\nend\nlocal function setHover() end\n"
    assert uses_before_local(deeper) == [("setHover", 2, 4, "if x then setHover() end")], uses_before_local(deeper)
    # A use above two declarations of one block gives one hit, with the nearest one.
    twice = "h()\nlocal function h() end\nlocal function h() end\n"
    assert uses_before_local(twice) == [("h", 1, 2, "h()")], uses_before_local(twice)
    # The syntax: a missing LuaJIT is a finding (it fails closed), never a pass.
    with tempfile.TemporaryDirectory() as temp:
        missing = os.path.join(temp, "no-luajit.exe")
        assert syntax_problem("E:/good.lua", clean, missing) == (
            "no LuaJIT at %s for the syntax check: set the key 'luajit' of local.json" % missing)
        # A file at the path that is not a program, and a program that is not LuaJIT (it fails
        # without LuaJIT's chunk name), are a finding that names the key, not a syntax fault.
        text_file = os.path.join(temp, "luajit.exe")
        with open(text_file, "w") as file:
            file.write("not a program\n")
        fault = syntax_problem("E:/good.lua", clean, text_file)
        assert fault.startswith("LuaJIT at %s did not run (" % text_file), fault
        assert fault.endswith("): set the key 'luajit' of local.json"), fault
        fault = syntax_problem("E:/good.lua", clean, sys.executable)
        assert fault.startswith("LuaJIT at %s did not run (" % sys.executable), fault
        assert fault.endswith("): set the key 'luajit' of local.json"), fault
    exe = luajit()
    if os.path.isfile(exe):
        assert syntax_problem("E:/good.lua", clean, exe) is None, syntax_problem("E:/good.lua", clean, exe)
        # The null of the syntax: a table left open; LuaJIT names the file and the line.
        fault = syntax_problem("E:/bad.lua", "local x = {\nlocal y = 1\n", exe)
        assert fault == "the Lua syntax: bad.lua:2: unexpected symbol near 'local'", fault
        # A long path keeps the line number (its base name is the chunk's name), and a name with a
        # quote reaches LuaJIT whole.
        deep = "C:/" + "a-folder-of-a-long-path/" * 6 + "batch.lua"
        assert syntax_problem(deep, "\n\nx =", exe) == "the Lua syntax: batch.lua:3: unexpected symbol near '<eof>'", syntax_problem(deep, "\n\nx =", exe)
        assert 'a "b".lua:1:' in syntax_problem('C:/a "b".lua', "x =", exe)
        # A byte 0x1A in a string is valid Lua: the binary read keeps the rest of the file.
        assert syntax_problem("E:/sub.lua", "local s = '\x1a'\nreturn s\n", exe) is None
        # A broken file read from its binary copy is still a syntax fault with its line.
        assert syntax_problem("E:/bad.lua", "x =", exe) == "the Lua syntax: bad.lua:1: unexpected symbol near '<eof>'"
        # The runner's view: each finding is a line, and the clean batch has none.
        assert problems("E:/good.lua", clean, exe) == []
        assert problems("E:/bad.lua", USE_ABOVE, exe) == [
            "the local function setHover is used on line 5 and declared on line 10 (a use above its declaration reads the "
            "global setHover): setHover(false)"], problems("E:/bad.lua", USE_ABOVE, exe)
    else:
        print("lua_checks self-test: the LuaJIT cases are skipped, no LuaJIT at %s" % exe)
    print("lua_checks self-test: ok")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if not argv:
        print(__doc__)
        return 2
    exe = luajit()
    found = 0
    for path in argv:
        with open(path, encoding="utf-8", errors="replace") as file:
            text = file.read()
        for line in problems(path, text, exe):
            print("%s: %s" % (path, line))
            found += 1
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
