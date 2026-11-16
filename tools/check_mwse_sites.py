"""The patch sites of an MWSE.dll against the plugin's ports, before the DLL goes to a test install; and the probe of the
MWSE features that the plugin needs.

    python -I tools/check_mwse_sites.py <MWSE.dll>              # exit code 0 when every site is followed
    python -I tools/check_mwse_sites.py --features <MWSE.dll>   # exit code 0 when the DLL has every required feature
    python -I tools/check_mwse_sites.py --features --mge <render-link tree> <MWSE.dll>   # and what that tree's d3d8 needs
    python -I tools/check_mwse_sites.py --self-test             # the tests of the output filters: no cargo

The check of the sites runs the unit test `mwse_sites::tests::every_mwse_site_in_a_port_is_followed` of the plugin
(src/mwse_sites.rs) with NIRENDER_MWSE_DLL set to the DLL, and prints its lines: each place that
the DLL names in a function of the table, the failures, the stale lines of
tools/mwse_sites.baseline, the reach of the decode of the DLL's code, and a count. A failure names the site, for example "MWSE.dll names
0x6a9d83, NiDX8Renderer::deleting_dtor+0x3, push x1, in the stolen bytes of ...: the detour will
refuse the renderer". The rules and the limits: the header of src/mwse_sites.rs (Brief 22,
2026-11-10). It fails on any new MWSE patch in a ported function.

--features asks another question (W406): does this MWSE.dll have what this plugin needs? It runs the unit test
`mwse_sites::tests::every_required_mwse_feature_is_present` (the list MWSE_FEATURES of src/mwse_sites.rs) and prints one
line for each feature: "feature: <name>: present (...)", "MISSING: <name>: ...: the plugin needs it" for a required feature
that the DLL lacks, "absent: <name>: ..." for one that the plugin can run without; then "md5: <md5> <path>". Exit code 4
(MISSING_FEATURE_EXIT) when a required feature is absent: the plugin must not go into an install with this DLL. Any other
failure (a build error, no file, no PE file) is exit code 1.

--mge <tree> adds what the d3d8.dll of a render-link tree needs (W495). The d3d8 of render-link 76918d5 and later sends
MWSE's native request 7 (MWSE_NATIVE_REQUEST_MGE_API) at its start, and without it every start with the renderer on becomes
the plain game. The plugin does not send it, so the probe's feature "mge api request" (the two log texts of MWSE's answer to
the request in the DLL's data) is not required by the plugin. When a .cpp file under d3d8/cpp of the tree names
MWSE_NATIVE_REQUEST_MGE_API and the DLL lacks the feature, the tool adds the line "MISSING: native request 7: <path> (md5
<md5>) has no native request 7 (MWSE_NATIVE_REQUEST_MGE_API), which the d3d8 of <tree> sends: MWSE too old; missing
<texts>" and gives exit code 4. Each --mge run first prints one "d3d8: " line with what the source check found: the .cpp
files that send the request, or the count of .cpp files and the other files that only name the constant (a header). A tree
with no d3d8/cpp folder is exit code 1. The check reads the tree's files, not a
commit: the merge check runs it on a sibling with no tracked change.

Both run the plugin's tests, which compile when they are not built: a compile, so ask for a build place first (slot.py
take build).
"""
import argparse
import hashlib
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from common import tests_ran  # noqa: E402
TEST = "mwse_sites::tests::every_mwse_site_in_a_port_is_followed"
FEATURES_TEST = "mwse_sites::tests::every_required_mwse_feature_is_present"
# The lines of the test; cargo's own lines are left out. "reach: " is the line of the decode of the code: how much of
# .text it reached, the blocks dropped as data and the overlaps. A fall from one build to the next shows there.
MARKS = ("ok: ", "FAIL: ", "stale: ", "reach: ")
# The lines of the probe of the features, and the line of the DLL's md5 that this tool adds.
FEATURE_MARKS = ("feature: ", "MISSING: ", "absent: ")
MISSING = "MISSING: "
MD5 = "md5: "
# The exit code when the DLL lacks a feature that the plugin needs. merge_step.py and build_set.py read it (W521: the
# maker probes a set's MWSE.dll before the seal; deploy_rest.py probes no more).
MISSING_FEATURE_EXIT = 4
# The feature of MWSE's native request 7 (MGE_API_REQUEST of src/mwse_sites.rs): the plugin runs without it, and a d3d8
# source that sends the request makes it required (--mge, W495).
MGE_API_FEATURE = "mge api request"
MGE_API_CONSTANT = "MWSE_NATIVE_REQUEST_MGE_API"
# The name of the MISSING line of the request. merge_step.py names it in the NOT DEPLOYED line.
MGE_API_NEED = "native request 7"
D3D8_SOURCE = ("d3d8", "cpp")
# The mark of the line of the source check. build_set.py prints it with the lines of the probe.
D3D8 = "d3d8: "


def report_lines(output):
    """The lines of the check in the output of cargo test: the places, the stale lines, the reach and the count."""
    return [line for line in output.splitlines() if line.startswith(MARKS) or " places in ported or census functions, " in line]


def feature_lines(output):
    """The lines of the probe of the features in the output of cargo test."""
    return [line for line in output.splitlines() if line.startswith(FEATURE_MARKS)]


def features_exit(returncode, lines):
    """The exit code of --features: 0 when the test passed; MISSING_FEATURE_EXIT when it failed on a MISSING line; else 1
    (the test did not get to judge the features)."""
    if returncode == 0:
        return 0
    return MISSING_FEATURE_EXIT if any(line.startswith(MISSING) for line in lines) else 1


def d3d8_request_sources(mge):
    """What the d3d8 source of the render-link tree `mge` holds of MWSE_NATIVE_REQUEST_MGE_API: (the .cpp files that name it,
    the other files that name it, the number of .cpp files), as paths below d3d8/cpp; None when the tree has no d3d8 source.
    Only a .cpp file sends the request: d3d8 keeps a copy of MWSE's header, which defines the constant and sends nothing."""
    source = os.path.join(mge, *D3D8_SOURCE)
    if not os.path.isdir(source):
        return None
    senders, others, count = [], [], 0
    for folder, _, names in os.walk(source):
        for name in sorted(names):
            path = os.path.join(folder, name)
            sender = name.endswith(".cpp")
            count += sender
            with open(path, encoding="utf-8", errors="replace") as file:
                if MGE_API_CONSTANT in file.read():
                    (senders if sender else others).append(os.path.relpath(path, source).replace(os.sep, "/"))
    return senders, others, count


def d3d8_line(mge, sources):
    """The line of what the source check found, printed by each --mge run (verdict 412): the files that send the request, or
    the count of .cpp files and the other files that only name the constant."""
    senders, others, count = sources
    if senders:
        return "%s%s sends native request 7 (%s)" % (D3D8, mge, ", ".join(senders))
    named = "; named only in %s" % ", ".join(others) if others else ""
    return "%s%s sends no native request 7 (0 of %d .cpp files under d3d8/cpp name %s%s)" % (D3D8, mge, count, MGE_API_CONSTANT, named)


def mge_api_request_line(lines, dll, md5, mge):
    """The MISSING line of native request 7 when the probe's lines do not show the feature as present, else None. The texts
    that the DLL's data lacks come from the probe's "absent: " line, so that an MWSE that rewords an answer is easy to see."""
    if any(line.startswith("feature: %s: " % MGE_API_FEATURE) for line in lines):
        return None
    absent = next((line for line in lines if line.startswith("absent: %s: " % MGE_API_FEATURE)), None)
    texts = absent.split(", missing ", 1)[1].rsplit(": the plugin runs without it", 1)[0] if absent and ", missing " in absent else "no probe line"
    return "%s%s: %s (md5 %s) has no native request 7 (%s), which the d3d8 of %s sends: MWSE too old; missing %s" % (
        MISSING, MGE_API_NEED, dll, md5, MGE_API_CONSTANT, mge, texts)


def md5_of(path):
    digest = hashlib.md5()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def self_test():
    output = "\n".join([
        "   Compiling nirender v0.3.6",
        "running 1 test",
        "ok: 0x6a9aed, NiDX8Renderer::ctor+0x7d, push x3 (BODY)",
        "FAIL: MWSE.dll names 0x6a9d83, NiDX8Renderer::deleting_dtor+0x3, push x1, in the stolen bytes of NiDX8Renderer::deleting_dtor: the detour will refuse the renderer",
        "stale: the baseline has 0x6aff07 mov-r32 x1, and the DLL does not",
        "reach: 18337040 of 19088896 bytes of .text (96.1 %, 536 of them weak), 1055138 blocks, 889 dropped as data, 11 overlaps",
        "x.dll: 105 places in ported or census functions, 1 failed, 1 stale baseline lines",
        "test mwse_sites::tests::every_mwse_site_in_a_port_is_followed ... FAILED",
    ])
    lines = report_lines(output)
    assert len(lines) == 5 and lines[1].startswith("FAIL: MWSE.dll names 0x6a9d83") and lines[3].startswith("reach: "), lines

    # The features: the lines of the probe; a required feature that is absent gives exit code 4, one that the plugin can
    # run without gives 0 with its line, and a failure with no MISSING line (a build error) gives 1.
    missing = "\n".join([
        "   Compiling nirender v0.3.6",
        "MISSING: count locks: 0 of 40 count lock sites in the data of MWSE.dll, missing 0x6fe39f, ...: the plugin needs it",
        "absent: b: 0 of 1 values in the data of MWSE.dll, missing 0x12345678: the plugin runs without it",
        "test mwse_sites::tests::every_required_mwse_feature_is_present ... FAILED",
    ])
    lines = feature_lines(missing)
    assert [line.split(":")[0] for line in lines] == ["MISSING", "absent"], lines
    assert features_exit(101, lines) == MISSING_FEATURE_EXIT
    optional = feature_lines("feature: count locks: present (40 of 40 count lock sites in the data of MWSE.dll)\n"
                             "absent: b: 0 of 1 values in the data of MWSE.dll, missing 0x12345678: the plugin runs without it\n")
    assert len(optional) == 2 and features_exit(0, optional) == 0, optional
    assert features_exit(101, feature_lines("error[E0425]: cannot find value `x`\n")) == 1, "a build error is no missing feature"
    assert features_exit(101, []) == 1

    # W495: native request 7. The present line gives no MISSING line; an absent line gives one that names the DLL, its md5,
    # the request, the tree and the texts that the DLL lacks; no line of the feature at all is no pass either.
    present = feature_lines("feature: mge api request: present (2 of 2 answer texts in the data of MWSE.dll)\n")
    assert mge_api_request_line(present, "D:/x/MWSE.dll", "a" * 32, "C:/r") is None
    absent = feature_lines("absent: mge api request: 0 of 2 answer texts in the data of MWSE.dll, missing \"keeps MGE XE's API in use.\", "
                           "\"withdraws MGE XE's API: the mge calls do nothing from now on.\": the plugin runs without it\n")
    line = mge_api_request_line(absent, "D:/x/MWSE.dll", "a" * 32, "C:/r")
    assert line == ("MISSING: native request 7: D:/x/MWSE.dll (md5 %s) has no native request 7 (MWSE_NATIVE_REQUEST_MGE_API), which the "
                    "d3d8 of C:/r sends: MWSE too old; missing \"keeps MGE XE's API in use.\", \"withdraws MGE XE's API: the mge calls do "
                    "nothing from now on.\"" % ("a" * 32)), line
    assert mge_api_request_line([], "D:/x/MWSE.dll", "a" * 32, "C:/r").endswith("; missing no probe line")
    # The source check: a .cpp that names the constant sends the request; the copy of MWSE's header does not; a tree with
    # no d3d8/cpp folder is unknown.
    # Each --mge run prints what it found (verdict 412): a request named only through a header is no sender, and its line
    # names that header.
    with tempfile.TemporaryDirectory() as root:
        assert d3d8_request_sources(root) is None
        source = os.path.join(root, *D3D8_SOURCE)
        os.makedirs(os.path.join(source, "mwse"))
        with open(os.path.join(source, "mwse", "MWSENativeApi.h"), "w", encoding="utf-8") as file:
            file.write("#define MWSE_NATIVE_REQUEST_MGE_API 7u\n")
        with open(os.path.join(source, "main.cpp"), "w", encoding="utf-8") as file:
            file.write("int main() { return 0; }\n")
        sources = d3d8_request_sources(root)
        assert sources == ([], ["mwse/MWSENativeApi.h"], 1), sources
        assert d3d8_line("C:/r", sources) == ("d3d8: C:/r sends no native request 7 (0 of 1 .cpp files under d3d8/cpp name "
                                              "MWSE_NATIVE_REQUEST_MGE_API; named only in mwse/MWSENativeApi.h)"), d3d8_line("C:/r", sources)
        assert d3d8_line("C:/r", ([], [], 3)) == "d3d8: C:/r sends no native request 7 (0 of 3 .cpp files under d3d8/cpp name MWSE_NATIVE_REQUEST_MGE_API)"
        with open(os.path.join(source, "mwse", "mwsenative.cpp"), "w", encoding="utf-8") as file:
            file.write("return request(MWSE_NATIVE_REQUEST_MGE_API, &packet, what);\n")
        sources = d3d8_request_sources(root)
        assert sources == (["mwse/mwsenative.cpp"], ["mwse/MWSENativeApi.h"], 2), sources
        assert d3d8_line("C:/r", sources) == "d3d8: C:/r sends native request 7 (mwse/mwsenative.cpp)", d3d8_line("C:/r", sources)
    # W527 (W524 F-G-7): cargo's own text of a filter that matches no test, exit code 0. The null: before W527 exit code
    # 0 was "mwse sites: ok".
    none = "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 412 filtered out; finished in 0.00s\n\n"
    assert sites_problem(0, none, report_lines(none)) == "the test ran 0 times"
    count = "C:/x/MWSE.dll: 37 places in ported or census functions, 0 failed, 0 stale baseline lines; 1.2 s\n"
    one = "\nrunning 1 test\n%stest %s ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 411 filtered out\n" % (count, TEST)
    assert sites_problem(0, one, report_lines(one)) is None, report_lines(one)
    assert sites_problem(0, one.replace(count, ""), report_lines(one.replace(count, ""))) == "the test printed no count line"
    assert sites_problem(101, one, report_lines(one)) == "the test failed (exit code 101)"
    print("check_mwse_sites self-test: ok")


def run_test(test, dll):
    env = dict(os.environ, NIRENDER_MWSE_DLL=dll)
    done = subprocess.run(["cargo", "test", "--release", "--lib", test, "--", "--exact", "--nocapture"], cwd=REPO, env=env,
                          capture_output=True, text=True, encoding="utf-8", errors="replace")
    return done.returncode, done.stdout + done.stderr


def sites_problem(code, output, lines):
    """Why the run of the sites test does not pass, or None. Exit code 0 is a pass only with the test's own count line and
    at least one test run (common.tests_ran): a filter that matches no test exits with 0 and judges nothing (W527, W524
    F-G-7)."""
    if code != 0:
        return "the test failed (exit code %d)" % code
    if tests_ran(output) == 0:
        return "the test ran 0 times"
    if not any(" places in ported or census functions, " in line for line in lines):
        return "the test printed no count line"
    return None


def check_sites(dll):
    code, output = run_test(TEST, dll)
    lines = report_lines(output)
    for line in lines:
        print(line)
    problem = sites_problem(code, output, lines)
    if problem:
        # The test did not get to its count: a build error, the DLL could not be read, or no test ran.
        print(output[-3000:])
    print("mwse sites: %s" % ("ok" if not problem else "FAILED: %s" % problem))
    return 1 if problem else 0


def check_features(dll, mge=None):
    sources = d3d8_request_sources(mge) if mge else None
    if mge and sources is None:
        print("no d3d8 source in %s: the needs of its d3d8.dll are unknown" % os.path.join(mge, *D3D8_SOURCE))
        print("mwse features: FAILED")
        return 1
    if sources:
        print(d3d8_line(mge, sources))
    needs_request = bool(sources and sources[0])
    code, output = run_test(FEATURES_TEST, dll)
    lines = feature_lines(output)
    md5 = md5_of(dll)
    request = mge_api_request_line(lines, dll, md5, mge) if needs_request else None
    for line in lines + ([request] if request else []):
        print(line)
    print("%s%s %s" % (MD5, md5, dll))
    result = features_exit(code, lines)
    if request and result == 0:
        result = MISSING_FEATURE_EXIT
    if result == 1:
        # The test did not judge the features: a build error, or the DLL could not be read.
        print(output[-3000:])
    print("mwse features: %s" % {0: "ok", MISSING_FEATURE_EXIT: "MISSING"}.get(result, "FAILED"))
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dll", nargs="?", help="the MWSE.dll to check")
    ap.add_argument("--features", action="store_true", help="probe the MWSE features that the plugin needs, not the sites")
    ap.add_argument("--mge", help="with --features: the render-link tree whose d3d8.dll goes beside the DLL; its needs count too")
    ap.add_argument("--self-test", action="store_true", help="the tests of the output filters: no cargo")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.dll:
        ap.error("name the MWSE.dll")
    dll = os.path.abspath(args.dll)
    if not os.path.isfile(dll):
        print("no file %s" % dll)
        return 2
    if args.mge and not args.features:
        ap.error("--mge goes with --features")
    return check_features(dll, args.mge and os.path.abspath(args.mge)) if args.features else check_sites(dll)


if __name__ == "__main__":
    sys.exit(main())
