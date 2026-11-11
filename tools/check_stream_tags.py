"""Checks the stream's tags and record sizes: the plugin's writer (`src/capture.rs`) against the
helper's reader (`host/src/tag.rs`).

    python tools/check_stream_tags.py              # the check
    python tools/check_stream_tags.py --list       # also each record's parts and their lengths
    python tools/check_stream_tags.py --self-test  # the tests of the parsing, with the nulls

1. The tags: each `pub const NAME: u32 = N;` of the module `tag` of capture.rs is a constant of
   the same name and number in tag.rs, and the reverse; tag.rs `KNOWN` holds each.
2. The sizes: for each tag that tag.rs `least_length` gives a length other than 0, the fixed part
   of each record that capture.rs writes with that tag (`write`, `write_record`, and the
   `records.push` of the frame end) has that length. The fixed part is the sum of the parts
   whose length the source fixes:
   - `words(&[a, b, c])`: 4 bytes for each item;
   - `words(x)`, `words(&x)` or `&x`, where `x` is an array: a parameter or a local with an array
     type, an array literal, a field of a struct, or the result of a function of capture.rs with
     an array type; the length may be a constant of capture.rs (the heads of `tag`);
   - `words(&x.map(..))` of such an array; `bytes_of(x)` of a struct of `src/link.rs` with a size
     assertion; `x.to_le_bytes()` of an integer; `[a, b].concat()` of such parts;
   - a part of a local list of parts (`tables[i]`, `vec![..]`, `[..]`).
   Any other part is variable. A tag of SIZE_EXCEPTIONS is not compared; the check prints why.
   A variable part adds 0 to the fixed part. So the check does not find an extra part that it
   cannot read (for example a call `&crate::link::four()`) in the record of a fixed-length tag:
   that record still passes.
   A tag that capture.rs writes and least_length does not name is not compared: the reader
   takes the whole record with its own check.

The exit code is 1 when a check fails.
"""
import argparse
import ast
import operator
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402

WRITER = os.path.join(REPO, "src", "capture.rs")
LINK = os.path.join(REPO, "src", "link.rs")
READER = os.path.join(REPO, "host", "src", "tag.rs")
# The constants that least_length names by path, and their files.
READER_MODULES = {"crate::effect_turn": os.path.join(REPO, "host", "src", "effect_turn.rs")}

# The records whose fixed part the writer takes from a value made at run time, by the tag and the
# function of capture.rs that writes it, so this check cannot read it; the reader checks each.
RUN_TIME = {
    ("HEADER", "tee"): "the header that the state keeps: header_record made it, and the check reads header_record",
    ("GAMMA_RAMP", "write_kept"): "the ramp that gamma_ramp kept: gamma_ramp takes it as [u8; GAMMA_RAMP_BYTES]",
    ("EFFECT_CHAIN", "effect_chain"): "the chain of effect_chain.rs read_chain, of a length of its links; effect_turn.rs of the "
                                      "helper checks the head, the links and the model",
}
# The tags whose reader takes a head of a fixed length, and the rest by a word of that head (a
# kind, a flag, a count) with its own check: a record of such a tag may be longer than
# least_length, never shorter.
LONGER = {
    "HEADER": "the stream format and the build id after the abilities: the handshake reads them by the record's length",
    "FRAME_EVENT": "the player reads the kind and the first value; the second is for the file tools",
    "OBJECT": "the player reads the kind and the class; the flags and the handles are for the file tools",
    "INSTANCE_DRAW": "the count of the lights follows the effects and the fog of the flags: tag::needed reads it",
    "CALL": "the arguments after the head of a call: tag::needed takes them by the kind of the call",
}
INTEGER_BYTES = {"u8": 1, "i8": 1, "u16": 2, "i16": 2, "u32": 4, "i32": 4, "f32": 4, "u64": 8, "i64": 8, "f64": 8, "usize": 4}


def without_comments(text):
    """`text` without `//` comments (doc comments too); string literals with `//` are kept."""
    out = []
    for line in text.split("\n"):
        at, quoted = 0, False
        while at < len(line):
            if line[at] == '"' and (at == 0 or line[at - 1] != "\\"):
                quoted = not quoted
            elif not quoted and line.startswith("//", at):
                line = line[:at]
                break
            at += 1
        out.append(line)
    return "\n".join(out)


def closing(text, at):
    """The index of the bracket that closes the one at `at`."""
    pairs = {"(": ")", "[": "]", "{": "}"}
    stack = []
    for i in range(at, len(text)):
        c = text[i]
        if c in pairs:
            stack.append(pairs[c])
        elif c in ")]}":
            if not stack or stack.pop() != c:
                raise ValueError("unbalanced at %d" % i)
            if not stack:
                return i
    raise ValueError("no close for %d" % at)


def split_top(text):
    """The items of a comma list, split at the commas outside brackets."""
    items, depth, start = [], 0, 0
    for i, c in enumerate(text):
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            items.append(text[start:i].strip())
            start = i + 1
    last = text[start:].strip()
    if last:
        items.append(last)
    return items


def evaluate(expression, names):
    """The value of an integer expression of numbers, `names` and + - * /."""
    operations = {ast.Add: operator.add, ast.Sub: operator.sub, ast.Mult: operator.mul, ast.FloorDiv: operator.floordiv, ast.Div: operator.floordiv}

    def value(node):
        if isinstance(node, ast.Constant) and isinstance(node.value, int):
            return node.value
        if isinstance(node, ast.Name) and node.id in names:
            return names[node.id]
        if isinstance(node, ast.BinOp) and type(node.op) in operations:
            return operations[type(node.op)](value(node.left), value(node.right))
        raise ValueError("not a number: %s" % ast.dump(node))
    text = re.sub(r"(\d)_(\d)", r"\1\2", expression)
    text = re.sub(r"\b(\d+)(?:u8|u16|u32|u64|usize|i32)\b", r"\1", text)
    text = re.sub(r"core::mem::size_of::<(\w+)>\(\)", lambda m: str(INTEGER_BYTES[m.group(1)]), text)
    return value(ast.parse(text.strip(), mode="eval").body)


def constants(text, kind="usize|u32"):
    """{name: value} of the `const NAME: usize = EXPR;` of `text` that evaluate."""
    found = {}
    for name, expression in re.findall(r"\bconst (\w+): (?:%s) = ([^;]+);" % kind, text):
        try:
            found[name] = evaluate(expression, found)
        except (ValueError, SyntaxError, KeyError):
            pass
    return found


# The reader.

def reader_tags(text):
    """{name: number} of the top-level tag constants of tag.rs, and the names of `KNOWN`."""
    tags = {name: int(value) for name, value in re.findall(r"^pub const (\w+): u32 = (\d+);", text, re.M)}
    known = re.search(r"pub const KNOWN: \[u32; \d+\] = \[([^\]]*)\]", text)
    return tags, [name.strip() for name in known.group(1).split(",") if name.strip()] if known else []


def least_lengths(text, modules):
    """{tag name: length} of the arms of tag.rs `least_length`; `modules` maps a module path to
    the text of its file, for the constants an arm names by path."""
    body_at = text.index("pub fn least_length")
    open_at = text.index("match tag {", body_at) + len("match tag ")
    body = without_comments(text[open_at + 1:closing(text, open_at)])
    names = {}
    for path, module in modules.items():
        for name, value in constants(module).items():
            names["%s::%s" % (path, name)] = value
    lengths = {}
    for arm in split_top(body):
        tag, expression = [part.strip() for part in arm.split("=>", 1)]
        if tag == "_":
            continue
        for path in sorted(names, key=len, reverse=True):
            expression = expression.replace(path, str(names[path]))
        lengths[tag] = evaluate(expression, {})
    return lengths


# The writer.

def writer_tags(text):
    """{name: number} of the constants of the module `tag` of capture.rs."""
    at = text.index("pub mod tag {") + len("pub mod tag ")
    module = text[at:closing(text, at) + 1]
    return {name: int(value) for name, value in re.findall(r"pub const (\w+): u32 = (\d+);", module)}


class Source:
    """capture.rs with what its parts need: its constants, its functions, its structs, and the
    sizes of the structs of link.rs."""

    def __init__(self, text, link):
        self.text = without_comments(text)
        self.constants = constants(self.text)
        self.structs = {}
        for name, body in re.findall(r"struct (\w+)(?:<[^>]*>)? \{([^}]*)\}", self.text):
            self.structs[name] = dict(re.findall(r"(\w+): ([^,]+),", body + ","))
        self.returns = dict(re.findall(r"fn (\w+)\([^)]*\) -> (\[[^\]]+\])", self.text))
        self.functions = []
        for match in re.finditer(r"\bfn (\w+)\s*(?:<[^>]*>)?\(", self.text):
            at = closing(self.text, match.end() - 1)
            body = self.text.find("{", at)
            if body < 0 or ";" in self.text[at:body]:
                continue
            self.functions.append((match.start(), closing(self.text, body), self.text[match.end():at], match.group(1), body))
        self.link_sizes = {name: int(size) for name, size in re.findall(r"assert!\(size_of::<(\w+)>\(\) == (\d+)\);", link)}

    def function_at(self, at):
        """(start, end, parameters, name, body) of the innermost function around `at`."""
        around = [f for f in self.functions if f[0] <= at <= f[1]]
        return max(around, key=lambda f: f[0])

    def tail_length(self, name, depth):
        """The bytes of the value that the function `name` of capture.rs ends with (its last
        expression), or None."""
        found = [f for f in self.functions if f[3] == name]
        if len(found) != 1:
            return None
        _, end, _, _, body = found[0]
        statements = split_top(self.text[body + 1:end].replace(";", ","))
        return self.part_length(statements[-1], end, depth + 1) if statements else None

    def array_length(self, text):
        """The bytes of an array type `[T; N]` (with `&`), or None. N may name a constant of
        capture.rs by a path (`tag::SHAPE_HEAD_WORDS`)."""
        match = re.fullmatch(r"&?(?:'\w+ )?\[(\w+); ([^\]]+)\]", text.strip())
        if not match or match.group(1) not in INTEGER_BYTES:
            return None
        try:
            length = re.sub(r"\b(?:\w+::)+(\w+)", r"\1", match.group(2))
            return INTEGER_BYTES[match.group(1)] * evaluate(length, self.constants)
        except (ValueError, SyntaxError):
            return None

    def binding(self, name, at):
        """What `name` is bound to before `at` in its function: ("type", text) or ("value", text),
        or None."""
        start, _, parameters, _, _ = self.function_at(at)
        scope = self.text[start:at]
        found = None
        for match in re.finditer(r"\blet (?:mut )?%s\b\s*(?::\s*([^=]+?))?\s*=\s*" % re.escape(name), scope):
            end = match.end()
            depth = 0
            while end < len(scope) and not (scope[end] == ";" and depth == 0):
                depth += scope[end] in "([{"
                depth -= scope[end] in ")]}"
                end += 1
            found = ("type", match.group(1)) if match.group(1) else ("value", scope[match.end():end].strip())
        for match in re.finditer(r"\blet (\w+) \{([^}]*)\} = (\w+)\(", scope):
            fields = [field.strip() for field in match.group(2).split(",")]
            returned = re.search(r"fn %s\([^)]*\) -> (\w+)" % match.group(3), self.text)
            if name in fields and returned and returned.group(1) in self.structs:
                found = ("type", self.structs[returned.group(1)][name].strip())
        if found:
            return found
        for parameter in split_top(parameters):
            if ":" in parameter and parameter.split(":", 1)[0].strip() == name:
                return ("type", parameter.split(":", 1)[1].strip())
        return None

    def name_length(self, name, at, item_bytes, depth=0):
        """The bytes that `name` holds before `at` (`item_bytes` for each item of a literal
        without a type), or None. A `vec![..]` counts its items: the least it holds."""
        bound = self.binding(name, at)
        if not bound or depth > 8:
            return None
        kind, text = bound
        if kind == "type":
            return self.array_length(text)
        literal = re.fullmatch(r"(?:vec!)?\[(.*)\]", text, re.S)
        if literal:
            repeat = re.fullmatch(r"(\d+)(u8|u16|u32|u64|f32)?\s*;\s*(.+)", literal.group(1).strip(), re.S)
            if repeat:
                each = INTEGER_BYTES[repeat.group(2)] if repeat.group(2) else item_bytes
                return each * evaluate(re.sub(r"\b(?:\w+::)+(\w+)", r"\1", repeat.group(3)), self.constants)
            return item_bytes * len(split_top(literal.group(1)))
        call = re.fullmatch(r"(\w+)\(.*\)", text, re.S)
        if call and call.group(1) in self.returns:
            return self.array_length(self.returns[call.group(1)])
        return self.part_length(text, at, depth + 1)

    def part_length(self, part, at, depth=0):
        """The bytes of a part of a record, or None when the part is variable."""
        part = re.sub(r"\.to_vec\(\)$", "", part.strip())
        if depth > 8:
            return None
        words = re.fullmatch(r"words\((.*)\)", part, re.S)
        if words:
            inner = words.group(1).strip()
            literal = re.fullmatch(r"&\[(.*)\]", inner, re.S)
            if literal and not re.search(r";", literal.group(1)):
                return 4 * len(split_top(literal.group(1)))
            mapped = re.fullmatch(r"&(\w+)\.map\(.*\)", inner, re.S)
            if mapped:
                return self.name_length(mapped.group(1), at, 4, depth)
            name = re.fullmatch(r"&?(\w+)", inner)
            return self.name_length(name.group(1), at, 4, depth) if name else self.part_length(inner, at, depth + 1)
        bytes_of = re.fullmatch(r"bytes_of\(&?(\w+)\)", part)
        if bytes_of:
            bound = self.binding(bytes_of.group(1), at)
            struct = bound and re.fullmatch(r"&(\w+)", bound[1].strip())
            return self.link_sizes.get(struct.group(1)) if struct else None
        integer = re.fullmatch(r"(\w+)\.to_le_bytes\(\)", part)
        if integer:
            bound = self.binding(integer.group(1), at)
            return INTEGER_BYTES.get(bound[1].strip()) if bound and bound[0] == "type" else None
        concat = re.fullmatch(r"\[(.*)\]\.concat\(\)", part, re.S)
        if concat:
            # The least length of the joined parts: a variable part adds nothing.
            return sum(self.part_length(p, at, depth + 1) or 0 for p in split_top(concat.group(1)))
        call = re.fullmatch(r"&?(\w+)\(.*\)", part, re.S)
        if call:
            if call.group(1) in self.returns:
                return self.array_length(self.returns[call.group(1)])
            return self.tail_length(call.group(1), depth)
        name = re.fullmatch(r"&?(\w+)", part)
        if name:
            return self.name_length(name.group(1), at, 1, depth)
        return None

    def parts(self, expression, at):
        """The parts of the record that `expression` gives `write`: a list literal, a local list
        of parts, or one part; each a (text, bytes or None)."""
        expression = expression.strip()
        literal = re.fullmatch(r"&\[(.*)\]", expression, re.S)
        if literal:
            items = split_top(literal.group(1))
        else:
            name = re.fullmatch(r"&?(\w+)", expression)
            bound = self.binding(name.group(1), at) if name else None
            listed = bound and bound[0] == "value" and re.fullmatch(r"(?:vec!)?\[(.*)\]", bound[1], re.S)
            items = split_top(listed.group(1)) if listed else [expression]
        resolved = []
        for item in items:
            index = re.fullmatch(r"(\w+)\[(\d+)\]", item)
            bound = self.binding(index.group(1), at) if index else None
            listed = bound and bound[0] == "value" and re.fullmatch(r"\[(.*)\]", bound[1], re.S)
            if listed:
                item = split_top(listed.group(1))[int(index.group(2))]
            resolved.append((item, self.part_length(item, at)))
        return resolved

    def records(self):
        """[(tag, line, function, parts)] of each record that capture.rs writes."""
        found = []
        for match in re.finditer(r"\b(?:write|write_record)\(\s*tag::(\w+)\s*,|records\.push\(\(\s*tag::(\w+)\s*,", self.text):
            tag = match.group(1) or match.group(2)
            open_at = match.start() + match.group(0).index("(")
            if match.group(2):
                open_at += 1
            items = split_top(self.text[open_at + 1:closing(self.text, open_at)])
            line = self.text.count("\n", 0, match.start()) + 1
            found.append((tag, line, self.function_at(match.start())[3], self.parts(items[1], match.start())))
        return found


def problems_of(writer_text, link_text, reader_text, modules, listing=False, run_time=None, longer=None):
    """The problems of the tags and the sizes, and the lines of `--list`; `run_time` and `longer`
    in place of RUN_TIME and LONGER (the self-test)."""
    run_time = RUN_TIME if run_time is None else run_time
    longer = LONGER if longer is None else longer
    problems, lines = [], []
    written, read = writer_tags(writer_text), None
    read, known = reader_tags(reader_text)
    for name in sorted(set(written) | set(read)):
        if name not in read:
            problems.append("tag %s (%d) of capture.rs is not in tag.rs" % (name, written[name]))
        elif name not in written:
            problems.append("tag %s (%d) of tag.rs is not in capture.rs" % (name, read[name]))
        elif written[name] != read[name]:
            problems.append("tag %s is %d in capture.rs and %d in tag.rs" % (name, written[name], read[name]))
    for name in read:
        if name not in known:
            problems.append("tag %s of tag.rs is not in KNOWN" % name)
    least = least_lengths(reader_text, modules)
    for name in least:
        if name not in read:
            problems.append("least_length names %s, which is no tag of tag.rs" % name)
    source = Source(writer_text, link_text)
    records = source.records()
    for tag, line, function, parts in records:
        fixed = sum(length for _, length in parts if length is not None)
        shown = "; ".join("%s = %s" % (re.sub(r"\s+", " ", text), "variable" if length is None else length) for text, length in parts)
        if listing:
            lines.append("capture.rs:%d %s (%s): fixed %d: %s" % (line, tag, function, fixed, shown))
        want = least.get(tag, 0)
        if want == 0:
            continue
        if (tag, function) in run_time:
            lines.append("%s of %s (capture.rs:%d) not compared: %s" % (tag, function, line, run_time[(tag, function)]))
            continue
        if fixed < want or (fixed > want and tag not in longer):
            problems.append("capture.rs:%d (%s) writes %s with a fixed part of %d bytes, and least_length of tag.rs is %d (the parts: %s)" % (
                line, function, tag, fixed, want, shown))
    for tag, function in run_time:
        if (tag, function) not in {(t, f) for t, _, f, _ in records}:
            problems.append("RUN_TIME names %s of %s, and capture.rs writes no such record" % (tag, function))
    for tag, length in least.items():
        if length and tag not in {t for t, _, _, _ in records}:
            problems.append("least_length gives %s %d bytes, and capture.rs writes no record of it" % (tag, length))
    return problems, lines, len(written), len(records)


def self_test():
    writer = """
pub mod tag {
    /// two words
    pub const PAIR: u32 = 1;
    pub const HEAD: u32 = 2;
    pub const HEAD_WORDS: usize = 3;
    pub const SCENE: u32 = 3;
    pub const LIST: u32 = 4;
    pub const END: u32 = 5;
    pub const CALL: u32 = 9;
    pub const HEADER: u32 = 11;
}
struct Parts<'a> {
    head: [u32; 2],
    rest: &'a [u8],
}
fn parts_of() -> Parts<'static> { todo!() }
fn picture() -> [u32; 5] { [0; 5] }
pub unsafe fn pair(a: u32, b: u32) {
    write(tag::PAIR, &[words(&[a, b]), bytes]);
}
pub unsafe fn head(head: &[u32; tag::HEAD_WORDS], name: &[u8]) {
    write(tag::HEAD, &[words(head), name]);
}
fn header_record(width: u32) -> Vec<u8> {
    let mut header = words(&[width, 1, 2, 3, 4]).to_vec();
    header.extend_from_slice(&id_bytes());
    header
}
pub unsafe fn start(width: u32) {
    let header = header_record(width);
    write(tag::HEADER, &[&header]);
}
pub unsafe fn tee() {
    let header = {
        s.header.clone()
    };
    write(tag::HEADER, &[&header]);
}
pub unsafe fn call(index: u32, args: &[u32]) {
    let mut head = vec![index, 0, 0, args.len() as u32];
    head.extend_from_slice(args);
    write(tag::CALL, &[words(&head), words(&[1, 2])]);
}
pub unsafe fn scene(scene: &SceneV2) {
    write(tag::SCENE, &[bytes_of(scene)]);
}
pub unsafe fn list(ids: &[u32]) {
    let size = [0u8; 12];
    let head = [1, 2];
    let tables = [words(&head), words(ids)];
    let Parts { head: own, rest } = parts_of();
    write(tag::LIST, &[tables[0], tables[1], &size]);
    write_record(tag::LIST, &[words(&picture()), rest], true);
}
pub unsafe fn end(start: u32, end: u32) {
    records.push((tag::END, [start.to_le_bytes(), end.to_le_bytes()].concat()));
}
"""
    link = "const _: () = assert!(size_of::<SceneV2>() == 88);\n"
    reader = """
pub const PAIR: u32 = 1;
pub const HEAD: u32 = 2;
pub const SCENE: u32 = 3;
pub const LIST: u32 = 4;
pub const END: u32 = 5;
pub const CALL: u32 = 9;
pub const HEADER: u32 = 11;
pub const KNOWN: [u32; 7] = [PAIR, HEAD, SCENE, LIST, END, CALL, HEADER];
pub fn least_length(tag: u32) -> usize {
    match tag {
        // the words
        PAIR => 8,
        HEAD => (crate::m::WORDS + 1) * 4,
        SCENE => 88,
        LIST => 20,
        END => 8,
        CALL => 16,
        HEADER => 16,
        _ => 0,
    }
}
"""
    modules = {"crate::m": "pub const WORDS: usize = 2;\n"}
    run_time = {("HEADER", "tee"): "the kept header"}
    longer = {"CALL": "the arguments", "HEADER": "the format"}

    def check(writer_text=writer, link_text=link, reader_text=reader, run=run_time, more=longer):
        return problems_of(writer_text, link_text, reader_text, modules, True, run, more)
    problems, lines, tags, records = check()
    assert problems == [], problems
    assert (tags, records) == (7, 9), (tags, records)
    assert len([line for line in lines if line.startswith("HEADER of tee (") and line.endswith("not compared: the kept header")]) == 1, lines
    # HEADER of start: 20 (header_record's last value, the words before .to_vec(); the build id
    # is not counted); CALL: 16 of vec![..], then 8.
    assert any("HEADER (start): fixed 20" in line for line in lines) and any("CALL (call): fixed 24" in line for line in lines), lines

    # The nulls: each must fail with its line.
    def fails(want, **changes):
        found = check(**changes)[0]
        assert found and want in found[0], (want, found)
    fails("tag LIST is 4 in capture.rs and 6 in tag.rs", reader_text=reader.replace("LIST: u32 = 4", "LIST: u32 = 6"))
    fails("tag END (5) of capture.rs is not in tag.rs", reader_text=reader.replace("pub const END: u32 = 5;\n", "").replace(", END,", ","))
    fails("tag HEAD of tag.rs is not in KNOWN", reader_text=reader.replace("[PAIR, HEAD,", "[PAIR,"))
    fails("(head) writes HEAD with a fixed part of 16 bytes", writer_text=writer.replace("HEAD_WORDS: usize = 3", "HEAD_WORDS: usize = 4"))
    fails("(pair) writes PAIR with a fixed part of 12 bytes", writer_text=writer.replace("words(&[a, b])", "words(&[a, b, a])"))
    fails("(scene) writes SCENE with a fixed part of 92 bytes", link_text=link.replace("88", "92"))
    fails("(list) writes LIST with a fixed part of 19 bytes", writer_text=writer.replace("let size = [0u8; 12];", "let size = [0u8; 11];"))
    fails("(list) writes LIST with a fixed part of 24 bytes", writer_text=writer.replace("-> [u32; 5]", "-> [u32; 6]"))
    fails("(end) writes END with a fixed part of 12 bytes", writer_text=writer.replace("start: u32", "start: u64"))
    # A longer record of a tag that is not on the list, a shorter one of a tag that is, a record
    # written at run time that is not on its list, and a list entry with no record.
    fails("(call) writes CALL with a fixed part of 24 bytes", more={"HEADER": "the format"})
    fails("(start) writes HEADER with a fixed part of 12 bytes", writer_text=writer.replace("[width, 1, 2, 3, 4]", "[width, 1, 2]"))
    fails("(tee) writes HEADER with a fixed part of 0 bytes", run={})
    fails("RUN_TIME names HEADER of gone", run={("HEADER", "tee"): "kept", ("HEADER", "gone"): "no such writer"})
    print("check_stream_tags self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", action="store_true", help="print each record's parts and their lengths")
    ap.add_argument("--self-test", action="store_true", help="the tests of the parsing, with the nulls")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    read = lambda path: open(path, encoding="utf-8").read()  # noqa: E731
    modules = {path: read(file) for path, file in READER_MODULES.items()}
    problems, lines, tags, records = problems_of(read(WRITER), read(LINK), read(READER), modules, args.list)
    for line in lines:
        print(line)
    for problem in problems:
        print("FAIL: " + problem)
    if not problems:
        print("stream tags ok: %d tags, %d records of capture.rs against tag.rs" % (tags, records))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
