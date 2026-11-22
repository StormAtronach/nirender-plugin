"""The checks of a merge, for the Coordinator, on the merged tree: one PASS or FAIL line for each stage.

    python -I tools/merge_step.py <repo> <commit> --base <tip> [--baseline <folder>] [--sibling-base <commit>] [--mwse <commit>] [--gate-list <file>] [--no-wait] [--dry-run]
    python -I tools/merge_step.py --quick [<tree>] [--sibling <tree>] [--base <commit>] [--no-wait] [--dry-run]
    python -I tools/merge_step.py --self-test    # the tests of the pure parts: no build, no game
    python -I tools/merge_step.py --open-deploys # the WITHHELD deploys that no deploy closed

--gate-list FILE (W567 part D) is the Reviewer's passed list of the planned picture changes of a merge, one picture
of the baseline folder for each line (validate_retained.py's --gate-list). The replay gate passes only when the
pictures that changed equal the list exactly, and the round's last stage, "accept the gate list" (validate_retained.py
accept-list), makes the listed pictures of the gate's run the accepted ones. As the last stage, it runs only when every
stage before it passed, so the next round is equal with no list and no second round. The list goes with the accepted
folder only (no --baseline), and its md5 is in the gate's verdict, the accept's line and the close line.

--quick (W567 part A) is the worker's check of its own tree before the hand-off: the fast stages of the tree's repo
(fast_stages) at its HEAD, with the stop at the first failure, and one line for the done report:

    QUICK PASS <repo> <commit10> (sibling <name> <commit10> | no sibling): N of N (<stages>); log <path>
    QUICK FAIL <repo> <commit10> (...): <stage> (<word>: <reason>); log <path>

The tree is the current folder without <tree>, and it must have no tracked change. A sibling that the stages read must
have none either, and the line names its commit (verdict 605). --base is the base of the checks
stage; without it, the merge base of HEAD and the shared branch (master, or feature/wgpu-renderer for render-link).
The sibling is the tree beside it without --sibling: a plugin tree with no render-link tree there checks its docs
without the helper (docs.py, W510), and a render-link tree needs a plugin tree, whose tools run its stages. It takes one
build place, under a label that does not go ahead of the queue, and never the game slot: no set, no deploy, no gate, no
ref move and no close line. The log is <tree>/target/quick-<commit10>.log (then -2, -3), not on E:, so a worker can run
it while a game runs (D297).

<repo> is the tree of nirender-plugin or MGE-XE-G7-render-link, at <commit> and with no tracked change. There is no
trial merge: the tool refuses another tree state. It takes the build slot for the whole of its work and releases it. It
waits in the queue of that slot; with --no-wait it stops with exit code 3. --dry-run prints the stages and takes nothing.

When a stage needs the game slot (the replay gate, the instance A/B, deploy rest: GAME_STAGES), the check takes the game slot and the whole
build side in one take, slot.py's `take both` (a lease of 4 hours), for the whole check (the owner's D161, D183). It never
holds one of them while it waits for the other (the owner's D208, after the deadlock of 14:37 on 2026-11-10): it waits in
both queues and holds nothing until both are free. A check with no such stage leaves the game slot free, so a game can run
beside it: none of its stages may then name a path on E: (a set, a copy to E:\\nirender-keep), and the tool refuses
such a stage list (only its own log and closes.tsv, small appends, go to E: then). The slots are released on every exit
path: a pass, a failing stage, an exception and Ctrl-C. While the stages run, a thread notes the game slot's work each
minute, as no game runs and the slot lapses after 15 idle minutes; a hold that lapses all the same is taken again before
the next stage only when it is free. --dry-run and --self-test take no slot (the self-test uses slot.py on a scratch state
folder). A hold by another session, or by an earlier run of this session, makes this run wait in the queues; with
--no-wait it stops with exit code 3 and holds nothing. Its takes have a place of their own beside the other places of
its session (slot.py's --another, W361, when the slot.py has it): a wait of the session for the game, or another
merge check, keeps its place.

<tip> is the tip of the shared branch before the merge. It is required: the parents of <commit> do not tell the change, as
the first parent misses a fast-forward and a branch that merged the shared branch. The tool refuses a <tip> that is newer
than <branch>@{1}, the tip before the last move of the branch, as such a diff skips a part of the merge. An older <tip> only
widens the diff. The tree must be on the branch, and a branch with no earlier tip is refused. A second run on one merge
gives the same answer.

After a later commit, <branch>@{1} moves past a merge that failed or had no run. So the ref refs/merge-step/<branch> keeps
the last commit that passed all stages, and the tool also refuses a <tip> newer than it. A branch with no such ref has only
the rule of <branch>@{1}: the tool cannot see a merge with no run before the first pass of a branch.

A FORMAT CHANGE (W527): the instance A/B exits with code 6 when the base set refuses the capture with a format code of
the helper and the build under test plays it, so the two builds read two formats and cannot be compared on one capture.
The exit code decides, never a line's text. The stage passes only with --evidence "instance A/B grass=FILE", a file that
exists; the verdict and the round's close line name the file's md5. Without the file the round fails. Any other NOT RUN
of the stage fails the round, with or without a file. The base is never the commit (base_problem).

The stages of nirender-plugin: the plugin's tests, the host's tests, the host's GPU tests (its ignored tests that need a
GPU, as below, with the tests that run only in a child process left out), clippy_baseline.py --check, checks.py plugin
--strict (a check that cannot run is NOT RUN, and the round fails, W527). The
stages of render-link: the tests of the helper and mge-config; the helper's GPU tests (cargo test -p mgeHost64 --
--ignored --test-threads=1: the tests marked #[ignore] that need a GPU, one at a time, run through "merge_step.py
--require-tests <command>", which fails when no test ran); clippy_baseline.py --check --mge <repo>, checks.py mge --strict
--mge <repo>. The diff from <tip> to <commit> adds these:

- for nirender-plugin, "helper tests for the host's text": cargo test -p mgeHost64, all of the package's tests, in the
  render-link tree of the pair (the sibling, at the commit that the stage's name has), when the change touches anything
  under host/src/ (W523, the audit's F-H-2). The helper joins text of the host crate into its own shaders (the world
  shader, the depth and the spot functions), and tests in several of its modules read that text, most of them through
  helper functions: a mark on the tests would miss some, so the whole package runs. The stage runs through
  "merge_step.py --require-tests <command>" and fails when no test ran;
- "build the set" (W521): every round that deploys (the gate's paths, or DEPLOY_PATHS of the plugin) first makes the
  sealed set of the commits under test and the round's MWSE commit with tools/build_set.py, in the plugin's tree and in
  the take of the round (--in-hold <the round's label>). The set is E:\\nirender-keep\\mgebuilds\\s-<nir8>-<rl8>-<mwse8>.
  The maker builds each part in its own trees under C:\\tmp\\buildset and takes a part that an earlier set holds. The
  round's MWSE commit is --mwse <commit>, else the commit that the record of every test install names; a round that
  makes a set with no known MWSE commit is refused before its take. Before the seal the maker probes the set's MWSE.dll
  for what the plugin needs (MWSE_FEATURES of src/mwse_sites.rs) and what the d3d8.dll of the set needs (native request
  7, W495). A missing feature (the maker's exit code 4) is the word WITHHELD on the stage, and the round fails (Q10):
  the stages that take the set (the sites of the set, deploy rest, the replay gate, the helper sources, the instance
  A/B) are NOT RUN, and the line names the missing feature and --mwse <an MWSE commit that has it>. Any other failure of
  the maker fails the round, and the same stages are NOT RUN. The maker needs 20 GB free on C:, and a set may be cleaned
  from E: between rounds: the next round makes it again;
- for nirender-plugin, "mwse sites": tools/check_mwse_sites.py, when the change touches the ports, the hooks, the
  install, the function table, the call sites or the baseline of the check (Brief 22). A new port over an MWSE patch
  fails. In a round with a set, the stage "mwse sites (the set)" reads the set's MWSE.dll, the one that the deploy
  writes, after the set and before the deploy. A change of the baseline alone deploys nothing: the stages
  "mwse sites <instance>" then read the MWSE.dll deployed in each test install (Morrowind75testing, Outlander-test);
- a d3d8 build and check_contract.py --mge when the change touches d3d8, mge-config or contract/;
- the replay gate, tools/validate_retained.py baseline with no game, on the round's set (--mge-build), when the change
  touches the host's source, the helper's renderer, the shaders of the assets (the source of the shaders in a build), or
  a manifest or a lock of the build. The path decides: a stream or a table change can move a picture too. The gate does
  not run when the round's set was not made;
- for nirender-plugin, "table order" (W535, verdict 595), right after the gate, when the change touches a file of the
  table's order (ORDER_PATHS): tools/validate_retained.py order on the round's set, each capture of the gate replayed
  with MGE_XE_TABLE_ORDER=grouped and =engine, 0 pixels between them, and a capture whose orders differ (else NOT
  SHOWN). The self-test fails when a tracked file under host/ or src/ outside ORDER_PATHS names the order's code
  (ORDER_NAMES), so the list cannot miss a reader;
- with the gate, the A/B stages of tools/instance_ab.py: `base set` makes, in the same take, the set of the current
  tips of the target branches of BOTH repos with the round's MWSE commit (a set that exists and passes its check is taken as it is), and
  `instance A/B grass` replays a capture of Outlander-test with the base set (--base-build), the round's set
  (--now-build) and the base set again, on the same data, and compares the pictures and the hash lines of the grass
  calls. NOT RUN is not a PASS: it counts as a stage that did not pass (exit code 5 of the tool). The base set is made
  at <tip> for the tree's repo and at the sibling's commit that passed with <tip>: the pair as it passed, so the A/B
  shows the change of both repos since that pass. The sibling's HEAD is not the base, as it can hold code that <tip>
  lacks (a render-link that needs a newer nirender fails the base set). When the base pair has no set (the round that
  passed <tip> made none, as a tools change makes no set), the set of an equal-parts ancestor serves (W567 part C,
  equal_parts_base): the same render-link and MWSE commits, and a nirender commit whose diff to <tip> touches no part input
  (build_set.NIRENDER_PART_INPUTS, checked first against the maker's dep-info files), the nearest by commits. Its parts are
  the base's but for their build ids, which the A/B does not read. The plan, the log and the close line name the set used,
  its commits and the diff's count, or why the set is built. A pass (against the accepted folder) keeps its
  pair in the sibling's repo, in the ref refs/merge-step-pair/<plugin|mge>/<commit>, when the sibling's HEAD did not
  move during the round. A base with no kept pair (a pass from before these refs) stops the tool: --sibling-base
  <commit> names the sibling's commit. It also takes the place of a kept pair. The sibling's commit must be an ancestor
  of the sibling's HEAD. The tool prints the option in the first line, and the base set's manifest names its commits;
- after a pass that moves refs/merge-step/<branch>, the sibling's own refs/merge-step/<branch> moves to the sibling's
  HEAD of the round only when this round ran and passed every stage that the sibling's own check of its diff since
  that ref would run, matched by folder and command (W370). Else it stays, and one line names the stages that did not
  run or pass, or why the sibling's stages cannot be known. It also stays while the sibling has tracked changes, at
  the start of the round or at the move, as the docs and contract stages read its working tree, not its HEAD; and
  when git refuses the update, as another check moved the ref meanwhile. A ref never moves past a diff whose
  stages did not run. A
  round of one repo does not run the fixed stages of the other (its tests and its checks), so a pair merge gets one
  check for each repo;
- each ref line of a round (its own ref moves from <old> to <new>, or stays at <old> with the reason; the pair, which
  starts "this round's pair:"; the line of the sibling, which starts "the sibling <name>:") goes to the console and,
  after the summary, into the round's log (W532);
- a plugin change that touches DEPLOY_PATHS (the plugin DLL's src/, build.rs, build_support/, the mod's Lua,
  contract/) and not the paths of the gate (W406) makes the set and deploys it, with no gate and no A/B: the replay gate
  cannot see these, as it never runs the plugin;
- the build of the dist alone, `cargo xtask dist`, when a render-link change touches xtask and not the paths of the gate:
  the replay cannot see it;
- with the set, `deploy rest` (tools/deploy_rest.py --set <the round's set>, the owner's D193: the test installs hold
  the latest build): after the set and before the replay gate it puts the set into both test installs as their resting
  build. It runs only when the set was made and every stage before it passed, and the gate's result does not stop it: a
  build that the gate rejects is deployed too. It never copies a pdb, an esp, mgeXE.toml or d3d9.dll, backs up each
  replaced file once for each tag, compares each copy by MD5, and prints the MD5 of d3d8.dll, numidium.exe,
  akulakhan.dll, MWSE.dll and lua51.dll. A set whose lua51.dll differs from an install's stops it (D 11.2): the
  Coordinator runs or approves that deploy by hand with --lua51-change. While Morrowind, numidium or Mod Organizer runs,
  the tool writes nothing and does not wait, and the stage prints "WITHHELD deploy rest (<n> s): <why>. To deploy this
  build alone, run in <folder>: <command>" (in the log, and again after the summary). That refusal does not fail the
  round: WITHHELD counts as passed, so a merge whose other stages passed still moves refs/merge-step. Any other failure
  of the tool is a FAIL. The stage runs also when the replay gate or the A/B stages fail: only the stages before it
  decide (the gate is red until the owner accepts a new baseline, and the owner tests the latest build, D193). The round
  holds the game slot (deploy rest is one of GAME_STAGES), so that the sets on E: are allowed (evidence_problem);
- the helper's sources (W528): after the gate, the stage "helper sources" keeps the source files of numidium.exe of
  each repo, read from the dep-info file of the set's build in the maker's render-link tree
  (target/x86_64-pc-windows-msvc/release/numidium.d), with the two commits, in
  E:\\nirender-keep\\merge-step\\helper-sources.txt. It reads the file only when the numidium.exe beside it is the set's
  (by MD5): a part that the maker took from another set leaves the list as it is, with a line. A later round plans the gate for a change of a
  listed file of its repo, beside the fixed list of gate paths. The plan prints the list's commits; with no list, or a
  list whose commit is not an ancestor of the base, it plans the gate and says why;
- the open deploys (W528): a round that passes with its deploy WITHHELD appends a WITHHELD line with both commits to
  E:\\nirender-keep\\merge-step\\deploys.tsv, at the same gate as its close; deploy_rest.py appends a DEPLOYED line after
  each deploy of a build. A WITHHELD line stays open until a DEPLOYED line names the same or newer commits of both
  repos. Each round prints the open ones at its start, and its close line ends with "open deploys: K" (with the oldest);

These stages also need a sibling with no tracked change. The first line gives the hash of the sibling.

--baseline names the folder that the gate compares with: the accepted folder without it, or a copy in the same form, as a
reference and not an acceptance. The first line names it. A pass against a named folder does not move the ref. The gate
writes nothing in it: its pictures go to <plugin>/target/merge-step-now, which each gate of this tool clears first, under the
build slot, so that a stale picture cannot compare.

The stages run in their order up to the first one that does not pass (W567 part B): each stage after it prints NOT RUN
with the name of that stage, and the round fails. A WITHHELD stage counts as passed. The fast stages (the tests, the GPU
tests, the clippy baseline, the checks, the docs) run first, so a fault of the code stops the round before the set, the
deploy and the gate. Exit code 1 on any failure. The output of each stage, the PASS, FAIL, NOT RUN or WITHHELD
line of each stage and the closing "P passed, W withheld of N stages" line are in
E:\\nirender-keep\\merge-step\\merge-step-<commit>.log, where the data rule on build output does not reach. A rerun for the same
commit writes merge-step-<commit>-2.log, then -3, and so on: it never overwrites the log of an earlier round. It pushes nothing.

A pass of the merge (every stage passed, against the accepted folder) closes the items that the merge subjects name (D180):
a subject that ends with "(W<n>)" or "(W<n> <part>)", e.g. "(W96)" or "(W341 step A)". The item of a part keeps its part
("W341 step A"): it closes that part, not the row, which can hold other parts that are still open (W357). Only a subject
without a part closes the whole row. A tail of decisions ", D<n>" or ", D<n>/D<m>/..." before the closing parenthesis is
no part: "(W363, D325)" closes W363 and "(W341 step B, D300)" closes "W341 step B" (W365). The subjects are those of each merge
commit on the first-parent path from <tip> to <commit> and of <commit> itself, so one check of a batch of merges closes
each of them (D183). A path with no merge commit with such a subject, to a <commit> that is no merge, gets a WARNING line
before the round: a fast-forward closes only the item of <commit>. Each item appends one line to
E:\\nirender-keep\\merge-step\\closes.tsv, which the Archivist reads in its batch. The columns are time (ISO, with the offset),
item, repo, commit, stages ("6 of 6"), log and subject; the file starts with a header line. A failed stage writes nothing, nor
does any other subject. A rerun of the same commit adds no line for an item that the file has for it. A reader that holds the
file makes the append try again, then give up with a line: a missing close never fails the check.

The gate needs what a diff cannot give:

- the two repos as siblings under their own names, as the helper's Cargo.toml names ../../nirender-plugin/host. A
  worktree with another name would build the host of another tree, so the gate refuses it;
- `cargo xtask dist` of the pair, as the gate replays with the helper and the shaders of a build. This is the longest stage;
- the sibling at the commit that the merge goes with: this tool does not know the other repo's tip;
- accepted pictures. A planned new look fails the gate until a baseline is accepted, and that is the owner's choice. The
  accepted pictures also hold the distant land and the mgeXE.toml of their time, from the testing instance.
"""
import argparse
import contextlib
import datetime
import hashlib
import importlib.util
import io
import itertools
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import types

# The modules of this folder, also under `python -I`.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_mwse_sites import MISSING, MISSING_FEATURE_EXIT  # noqa: E402
from deploy_rest import MWSE_DEPLOYS, REFUSED_EXIT  # noqa: E402
from instance_ab import NOT_RUN as NOT_RUN_CODE  # noqa: E402
import build_set  # noqa: E402  (the maker's trees and the round's set, W521)
import sets  # noqa: E402
from common import (DEPLOY_WITHHELD, DEPLOYS_FILE, FORMAT_CHANGE_EXIT, NOT_RUN_EXIT, append_deploys, deploys_line,  # noqa: E402
                    docs_place, git_env, local, main_checkout, open_deploys, open_deploys_text, tests_ran)
from make_play_set import BASELINE, HARNESS, WAIT_SECONDS, has_captures, load_slots, say, slot_asker  # noqa: E402
from mwse_tree import fallback_checks, installed_commit  # noqa: E402

PLUGIN_NAME = "nirender-plugin"
MGE_NAME = "MGE-XE-G7-render-link"
SLOT_MINUTES = 60
# The lease of the game slot: the stages of a round run for up to an hour on a slow day (the longest of 2026-11-09 ran 38
# minutes), and the lease does not renew while they run.
GAME_SLOT_MINUTES = 240
# How often a round notes the work of the game slot while its stages run (the slot's idle limit is 15 minutes).
GAME_NOTE_SECONDS = 60
LOG_FOLDER = "E:\\nirender-keep\\merge-step"
# The closing lines of the items (D180): the Archivist reads this file in its batch. A pass of the merge appends one line for
# each item that its merge subject names.
CLOSES_FILE = os.path.join(LOG_FOLDER, "closes.tsv")
# The cite check of the Frame Map (check_cites.py of the docs, D362), report only, after the round: a merge that changes a
# cited range names the claims in the log. It never fails the round. The repo name of the page for each kind of tree.
CITE_REPOS = {"plugin": "nirender", "mge": "render-link"}
CITE_TOOL_PARTS = ("docs", "Numidium Project", "scripts", "frame-map", "check_cites.py")
CITE_TIMEOUT_SECONDS = 300
CLOSES_HEADER = "time\titem\trepo\tcommit\tstages\tlog\tsubject"
# The item of a merge subject: the subject ends with "(W<n>)" or "(W<n> <part>)", the form of the Coordinator's merge
# subjects ("(W96)", "(W341 step A)"). The item is the text in the parentheses, the part with it: a row with parts
# closes one part at a time (W357). References can follow as a tail, each after ", ", which the item drops and which
# close nothing: decisions ("(W363, D325)" is W363, "(W340, D288/D289/D310)" is W340, W365; "(W520, D488, D489)" is
# W520), a row of another item ("(W527, W524 row 2)" is W527, W528), a risk or an E item. The part is lazy, so that the
# tail is not taken into it; any other comma stays in the part ("(W341 step B, commit 1)"). Any other subject names no
# item and closes nothing: a bare second item ("(W222, W226)": which one closes is not known) and a part after a comma
# ("(W229, crate half)": W229 whole would close). unread_item_lines names each such subject with a W number. A part that
# starts with "dropped" is no part: master has "Merge: log pipe follow-ups W219 and W221 (W220 dropped)".
TAIL_REFERENCE = r"(?:D\d+(?:/D\d+)*|W\d+ (?!dropped\b)[^(),]+?|risk \d+|E\d+)"
ITEM = re.compile(r"\((W\d+(?: (?!dropped\b)[^()]+?)?)(?:, " + TAIL_REFERENCE + r")*\)$")
LAST_PARENTHESES = re.compile(r"\(([^()]*)\)\s*$")
# The tries of an append to the closes file while a reader holds it, and the wait between them.
CLOSE_TRIES = 5
CLOSE_WAIT_SECONDS = 0.5
# The stage that makes the round's sealed set (tools/build_set.py --in-hold, W521, the plan's addendum 10): the commits
# under test and the MWSE commit of the round (the installs' commit, or --mwse). The deploy, the gate, the helper sources
# and the A/B take that set. The maker's MISSING FEATURE (its exit code 4) makes it WITHHELD, and the stages that take
# the set are NOT RUN: the round fails, and its line names --mwse (Q10).
SET_STAGE = "build the set"
# The stage that makes the set of the base pair for the A/B; the A/B replays the base set and the round's set.
INSTANCE_STAGE = "instance A/B grass"
BASE_STAGE = "base set"
# The folder of a set in a plan whose commits are not known (the self-test; main refuses such a round).
UNPLANNED_SET = "<the round's set: its commits are not known>"
# The option of build_set.py that runs the maker inside the round's take; stage_key leaves out its label.
IN_HOLD = "--in-hold"
# The check of MWSE's patch sites on the set's MWSE.dll, the DLL that the deploy writes (addendum 11).
SITES_SET_STAGE = "mwse sites (the set)"
# The stage that puts the round's set into both test installs (tools/deploy_rest.py --set, the owner's D193). It stands after
# the set and before the replay gate, and it runs only when every stage before it passed: the gate's result does not stop it.
DEPLOY_STAGE = "deploy rest"
# The exit code of deploy_rest.py when a game or Mod Organizer runs. That refusal does not fail the round: the stage counts as
# passed (WITHHELD), and the round prints one line for it, in the log and again after the summary. Any other failure of
# the stage (a fault of the copy, a missing source) is a FAIL.
DEPLOY_REFUSED_CODE = REFUSED_EXIT
# The "WITHHELD deploy rest" lines of the last run_stages: the deploys that did not happen.
NOT_DEPLOYED = []
# The evidence files of the round, by the stage's name, as (path, md5): the one way a stage with a FORMAT CHANGE passes
# (W527). Only the instance A/B can have one; main fills it from --evidence before the round.
EVIDENCE = {}
# The FORMAT CHANGE verdicts of the last run_stages that passed with an evidence file, for the round's close line.
FORMAT_CHANGES = []
EVIDENCE_STAGES = (INSTANCE_STAGE,)
# The four words of a stage's verdict (W524). PASS and WITHHELD count as passed: a WITHHELD stage found the code right and
# kept a deploy back (D193, W406), so it does not fail the round.
PASS_WORD, FAIL_WORD, NOT_RUN_WORD, WITHHELD_WORD = "PASS", "FAIL", "NOT RUN", "WITHHELD"
STAGE_WORD_ORDER = (PASS_WORD, WITHHELD_WORD, NOT_RUN_WORD, FAIL_WORD)
# The verdicts of the last run_stages, as (stage, word, reason), one for each stage: the close line names them (W528).
STAGE_WORDS = []
# The stage of the helper's tests that need a GPU (marked #[ignore]); `merge_step.py REQUIRE_TESTS <command>` runs one
# command and fails when no test ran.
GPU_STAGE = "helper GPU tests"
HOST_GPU_STAGE = "host GPU tests"
REQUIRE_TESTS = "--require-tests"
# What a plugin change must touch for the helper's tests in the sibling (HELPER_TEXT_STAGE): the host crate's source,
# all of it. The helper's shaders hold text of several of its files (shader.wgsl, depth.rs, spot.rs, the join of
# request.rs), and a list of them by hand goes stale (the audit's F-G-4).
HELPER_TEXT_PATHS = ("host/src/",)
HELPER_TEXT_STAGE = "helper tests for the host's text"
# What a change must touch for the gate, as paths from the root of each repo: a file, or a folder with its slash.
GATE_PATHS = {"plugin": ("host/src/", "Cargo.toml", "Cargo.lock", "host/Cargo.toml", "host/Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
                         "host/.cargo/config.toml", "tools/hook-fixtures/"),
              "mge": ("mgeHost64/src/renderer.rs", "mgeHost64/src/renderer/", "assets/Data Files/shaders/", "Cargo.toml", "Cargo.lock",
                      "mgeHost64/Cargo.toml", "mge-config/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml")}
# The files of the table's order (W535): the switch MGE_XE_TABLE_ORDER, the order and its digest lines (Hob's merge
# branch fix/w535-engine-order: the module and its `mod` line, the switch's read, the order's collection, the replay's
# digest lines, the report's counts). A change of one of them adds the stage ORDER_STAGE after the replay gate: each
# capture of the gate in both orders, 0 pixels between them (validate_retained.py order; the Architect's page
# w535-w538-order-and-origin section 1, verdicts 595 and 597). ORDER_NAMES are the names of that code: a tracked file
# under host/ or src/ outside this list that names one of them fails the self-test (`order_drift`), so the list cannot
# miss a new reader. README.md names the switch too; it is not code, and a change of it does not run the stage.
ORDER_PATHS = {"plugin": ("host/src/table_order.rs", "host/src/switches.rs", "host/src/retained.rs", "host/src/player/report.rs",
                          "host/src/cli.rs", "host/src/lib.rs"), "mge": ()}
ORDER_NAMES = ("MGE_XE_TABLE_ORDER", "table_order", "TableOrder", "collect_order", "order_keys", "add_to_digest", "digest_frame", "take_digest")
ORDER_STAGE = "table order"
D3D8_PATHS = {"plugin": ("contract/",), "mge": ("d3d8/", "mge-config/")}
# What a change must touch for a build and a deploy without the gate (W406): the plugin DLL and the mod's Lua, which the
# replay gate cannot see (it replays old captures with the helper and never runs the plugin), and the contract between
# the plugin and d3d8.dll. A render-link change deploys only with the gate.
DEPLOY_PATHS = {"plugin": ("src/", "build.rs", "build_support/", "mod/MWSE/mods/nirender/", "contract/"), "mge": ()}
# What a change must touch for the build of the dist alone, with no deploy: xtask itself, which the replay cannot see.
DIST_PATHS = {"plugin": (), "mge": ("xtask/",)}
# What a plugin change must touch for the check of MWSE's patch sites against the ports (check_mwse_sites.py, Brief 22):
# a new port, hook or follow rule can meet an MWSE patch. A round with a set checks the set's MWSE.dll (SITES_SET_STAGE);
# a change of the baseline alone deploys nothing, and its stages check each install's MWSE.dll.
MWSE_SITES_PATHS = ("src/ports/", "src/hook.rs", "src/install.rs", "src/gen/", "src/site.rs", "tools/mwse_sites.baseline")
# W528 item 1 (F-G-4): the helper's own source files plan the gate, beside GATE_PATHS. After the set of the gate, the
# stage HELPER_SOURCES_STAGE reads the dep-info file that cargo wrote beside numidium.exe in the maker's render-link tree,
# when the set's numidium.exe is that build (addendum 10 C5), and keeps the source files
# of each repo in HELPER_SOURCES_PATH, with the two commits of the build. GATE_PATHS keeps what dep-info does not name:
# the shaders under assets/, the manifests and lock files, the toolchain files. A new source file is declared in a
# file that is on the list (a mod line, an include, a path dependency), so its diff touches a listed path; neither
# build.rs of the helper (mgeHost64, host) reads a glob: they read the commit only.
HELPER_SOURCES_STAGE = "helper sources"
HELPER_SOURCES = "--helper-sources"
HELPER_SOURCES_PATH = os.path.join(LOG_FOLDER, "helper-sources.txt")
# The list that a plan reads: HELPER_SOURCES_PATH, or None in the self-test, whose scratch repos read no real list.
HELPER_SOURCES_FILE = HELPER_SOURCES_PATH
HELPER_DEP_INFO = os.path.join("target", "x86_64-pc-windows-msvc", "release", "numidium.d")
HELPER_COMMITS = "commits"
# The parts of a repo that are no source of the helper, though dep-info can name them: the build's own output and git.
NOT_SOURCES = ("target/", ".git/")
HARNESS_MOD = re.compile(r"[/\\]mods[/\\]Harness - [^/\\]*[/\\]", re.IGNORECASE)
# These move a build out of its tree. Cargo then returns 0, and the gate replays the old file of the build folder.
BUILD_MOVERS = ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "NIRENDER_PLUGIN_DIR")


def touches(paths, wanted):
    return any(path == item or (item.endswith("/") and path.startswith(item)) for path in paths for item in wanted)


# A name of the order's code as a word: `table_order` names the field, `stable_order` does not.
ORDER_WORD = re.compile(r"\b(?:%s)\b" % "|".join(ORDER_NAMES))


def order_drift(texts):
    """The files of `texts` ({path: text}, paths under host/ and src/ of the nirender tree) that name the order's code
    (ORDER_NAMES) and are not in ORDER_PATHS: a reader of the switch or of the order that the list misses."""
    return sorted(path for path, text in texts.items() if not touches([path], ORDER_PATHS["plugin"]) and ORDER_WORD.search(text))


def tracked_texts(repo, parts=("host/", "src/")):
    """{path: text} of the files that git tracks under `parts` of `repo`, as a reader of the code sees them."""
    listed = subprocess.run(["git", "-C", repo, "ls-files", "-z", "--", *parts], capture_output=True, check=True).stdout
    texts = {}
    for path in listed.decode("utf-8").split("\0"):
        if path:
            with open(os.path.join(repo, path), "rb") as file:
                texts[path] = file.read().decode("utf-8", "replace")
    return texts


def self_test_order_drift():
    """W535, verdict 595 C2: the order stage runs when a file of ORDER_PATHS changes, so a file outside the list that
    names the order's code fails here. The plant: a file outside the list with the switch's name is named; one in the
    list, one with a longer word and one under another folder are not. Then this tree: no file may drift."""
    planted = {"host/src/retained.rs": "fn collect_order() { table_order }", "host/src/frame.rs": "let o = MGE_XE_TABLE_ORDER;",
               "host/src/sort.rs": "fn stable_order() {}", "src/lua.rs": "TableOrder::Grouped", "host/src/table_order.rs": "pub enum TableOrder {}"}
    assert order_drift(planted) == ["host/src/frame.rs", "src/lua.rs"], order_drift(planted)
    assert order_drift({"host/src/player/report.rs": "table order digest"}) == []
    # The functions of the order and of the digest (verdict 597 C1): named in the replay's loop, fine; elsewhere, drift.
    assert order_drift({"host/src/cli.rs": "set.digest_frame(); set.take_digest()", "host/src/lib.rs": "mod table_order;",
                        "host/src/frame.rs": "self.add_to_digest(draw)", "host/src/view.rs": "order_keys(&shapes)"}) == ["host/src/frame.rs", "host/src/view.rs"]
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    drift = order_drift(tracked_texts(here))
    assert not drift, "files outside ORDER_PATHS name the table's order (%s): add them to ORDER_PATHS of merge_step.py" % ", ".join(drift)


def repo_kind(folder):
    """"plugin", "mge" or None, by what the tree holds."""
    if os.path.isfile(os.path.join(folder, "host", "src", "shader.wgsl")):
        return "plugin"
    return "mge" if os.path.isdir(os.path.join(folder, "mgeHost64")) else None


def sibling_of(kind, repo):
    return os.path.join(os.path.dirname(repo), MGE_NAME if kind == "plugin" else PLUGIN_NAME)


def gpu_command(plugin, package_args, skipped=()):
    """The command of a stage of ignored tests that need a GPU: cargo test --ignored, through the wrapper that fails when no
    test ran. One test thread: each module's GPU tests hold a lock of their own, so side by side they ran at the same
    time on one device and a timing test failed once under load. `skipped` names tests that stay out, by a part of their
    path (the tests of the host that dump or crash this process run alone in a child; their names end in _in_this_process)."""
    skip = [argument for name in skipped for argument in ("--skip", name)]
    return [sys.executable, "-I", os.path.join(plugin, "tools", "merge_step.py"), REQUIRE_TESTS, "cargo", "test", "--quiet", *package_args,
            "--", "--ignored", *skip, "--test-threads=1"]


def deployed_mwse_dlls(deploys=None):
    """The deployed MWSE.dll of each test install, as (install name, path), from `MWSE_DEPLOYS`. A missing file stays in
    the list, so that its stage fails and names it. A path in a `Harness - *` mod is refused: that is a run's DLL."""
    deploys = MWSE_DEPLOYS if deploys is None else deploys
    for name, path in deploys.items():
        if HARNESS_MOD.search(path):
            raise ValueError("mwse sites %s: %s is in a Harness mod, the DLL of a live run, not the deployed one" % (name, path))
    return list(deploys.items())


def under_test(kind, commit, sibling_commit):
    """The commits under test, {"plugin": sha, "mge": sha}, from the commit of the repo of `kind` and the sibling's commit
    at the start of the round; None without a sibling commit."""
    if not kind or not sibling_commit:
        return None
    return {kind: commit, "mge" if kind == "plugin" else "plugin": sibling_commit}


def helper_text_stage(plugin, mge, commits=None):
    """The stage of a plugin round that changes the host crate's source (W523, the audit's F-H-2): every test of the
    helper's package, in the render-link tree of the pair, through the wrapper that fails when no test ran. The name
    has the commit of that tree under test, when the round knows it."""
    name = HELPER_TEXT_STAGE + (" (render-link %s)" % commits["mge"][:10] if commits else "")
    return name, mge, [sys.executable, "-I", os.path.join(plugin, "tools", "merge_step.py"), REQUIRE_TESTS, "cargo", "test", "--quiet", "-p", "mgeHost64"]


def round_set(commits, mwse_commit):
    """The commits of a round's set, {repo of sets.REPOS: commit}, from a pair {"plugin": sha, "mge": sha} and the MWSE
    commit of the round, or None when one of them is not known."""
    if not commits or not mwse_commit:
        return None
    return {"nirender": commits["plugin"], "render-link": commits["mge"], "mwse": mwse_commit}


def set_folder_of(set_commits, root=None):
    """The folder of the set of `set_commits` (round_set) under the sets (sets.set_name: no patch, the usual kind), or
    UNPLANNED_SET when its commits are not known."""
    if not set_commits:
        return UNPLANNED_SET
    return os.path.join(sets.SETS if root is None else root, sets.set_name(set_commits))


def set_stage(name, plugin, set_commits, label):
    """A stage of the maker inside the round's take (build_set.py --in-hold <label>, addendum 10 C2)."""
    commits = set_commits or {repo: "unknown" for repo in sets.REPOS}
    return (name, plugin, [sys.executable, "-I", os.path.join("tools", "build_set.py"), "--nirender", commits["nirender"],
                           "--render-link", commits["render-link"], "--mwse", commits["mwse"], IN_HOLD, label or "merge_step"])


def maker_tree(repo):
    """The maker's tree of a repo of sets.REPOS (build_set.TREES)."""
    return os.path.join(build_set.TREES, build_set.TREE_NAMES[repo])


# W567 part D: the planned changes of the replay gate, [(path, md5)] of --gate-list, and the stage that accepts them
# as the round's last stage.
GATE_LIST = []
ACCEPT_LIST_STAGE = "accept the gate list"


def gate_list_of(value, baseline, exists=os.path.isfile):
    """[(path, md5)] of --gate-list, [] without it, or the reason it cannot be used. The accept writes the accepted
    folder, so the list goes only with it, not with a --baseline copy."""
    if not value:
        return []
    if not exists(value):
        return "the gate list %s is no file" % value
    if os.path.normcase(os.path.abspath(baseline)) != os.path.normcase(os.path.abspath(BASELINE)):
        return "--gate-list accepts the listed pictures into the accepted folder at the end of a pass, so it goes with no --baseline (here %s)" % baseline
    return [(os.path.abspath(value), file_md5(value))]


def gate_list_close(stages):
    """The part of a close line for a round's gate list (W567 part D): its file and md5, when the round accepted it."""
    if not GATE_LIST or not any(name == ACCEPT_LIST_STAGE for name, _, _ in stages):
        return []
    return ["gate list %s md5 %s accepted" % GATE_LIST[0]]


# The label of a --quick take: a tool's label (slot.py's rule of D504), and not the prefix of the merge checks
# ("merge_step:"), which goes ahead of the queue (D148). A worker's check waits its turn.
QUICK_LABEL = "merge_step.py: quick %s %s"
# The shared branch of each repo: a --quick round's checks compare with its merge base.
SHARED_BRANCHES = {"plugin": "master", "mge": "feature/wgpu-renderer"}


def fast_stages(kind, plugin, mge, base=None):
    """The fixed stages of every round of a repo of `kind`, as (name, folder, command): the tests, the GPU tests, the
    clippy baseline, the checks and the docs. They build and test, with no set, no game and no replay, so a round runs
    them first (W567 part B), and --quick runs them alone (part A). `base`: the checks stage compares with it. `mge` None:
    the docs stage of a plugin tree with no render-link tree skips the helper (docs.py, W510)."""
    tool = lambda name, *args: [sys.executable, "-I", os.path.join("tools", name), *args]  # noqa: E731
    on_base = ["--base", base] if base else []
    on_mge = ["--mge", mge] if mge else []
    if kind == "plugin":
        return [(TESTS_STAGE, plugin, ["cargo", "test", "--quiet"]),
                ("host tests", os.path.join(plugin, "host"), ["cargo", "test", "--quiet"]),
                (HOST_GPU_STAGE, os.path.join(plugin, "host"), gpu_command(plugin, [], ["_in_this_process"])),
                ("clippy baseline", plugin, tool("clippy_baseline.py", "--check")),
                ("checks", plugin, tool("checks.py", "plugin", "--strict", *on_base)),
                ("docs", plugin, tool("docs.py", *on_mge))]
    return [("helper tests", mge, ["cargo", "test", "--quiet", "-p", "mgeHost64", "-p", "mge-config"]),
            (GPU_STAGE, mge, gpu_command(plugin, ["-p", "mgeHost64"])),
            ("clippy baseline", plugin, tool("clippy_baseline.py", "--check", "--mge", mge)),
            ("checks", plugin, tool("checks.py", "mge", "--strict", "--mge", mge, *on_base)),
            ("docs", plugin, tool("docs.py", "--mge", mge))]


def stages_for(kind, repo, paths, baseline=BASELINE, tips=None, mwse_commit=None, mwse_dlls=None, commits=None, base=None, helper_paths=(),
               label=None, base_commits=None):
    """The stages of a merge, as (name, folder, command). The folder of the other repo is the sibling. `base_commits` (a
    round_set) names the base set when it is not the base pair's own (W567 part C, equal_parts_base). `base` is the base
    commit of the round: the checks stage compares with it (checks.py --base), as the tree is at the merge. `commits` are
    the commits under test, {"plugin": sha, "mge": sha}, and `mwse_commit` the MWSE commit of the round: with both, the
    round's set has a name, and every stage that takes it names its folder (W521). `tips` are the commits of the base
    pair, {"plugin": sha, "mge": sha}: with them, the gate is followed by the base set and the A/B. `mwse_dlls` are the
    (instance, MWSE.dll) of the "mwse sites" stages of a round with no set, by default those deployed now. `label` is the
    label of the round's take, which the maker's stages check (--in-hold). `helper_paths` are the helper's source files
    of this repo (helper_gate_paths), which plan the gate too; None plans it always."""
    python = sys.executable
    plugin = repo if kind == "plugin" else sibling_of(kind, repo)
    mge = repo if kind == "mge" else sibling_of(kind, repo)
    tool = lambda name, *args: [python, "-I", os.path.join("tools", name), *args]  # noqa: E731
    stages = fast_stages(kind, plugin, mge, base)
    gate = touches(paths, GATE_PATHS[kind]) or helper_paths is None or touches(paths, helper_paths)
    # A round that deploys makes a set; the gate's round always deploys.
    deploys = gate or touches(paths, DEPLOY_PATHS[kind])
    sites = kind == "plugin" and touches(paths, MWSE_SITES_PATHS)
    if kind == "plugin" and touches(paths, HELPER_TEXT_PATHS):
        stages.append(helper_text_stage(plugin, mge, commits))
    if sites and not deploys:
        # A change of the baseline alone: no set, and the installs keep the MWSE.dll of the last deploy (addendum 11).
        for instance, dll in deployed_mwse_dlls() if mwse_dlls is None else mwse_dlls:
            stages.append(("mwse sites " + instance, plugin, tool("check_mwse_sites.py", dll)))
    if touches(paths, D3D8_PATHS[kind]):
        stages.append(("d3d8 build", mge, ["cargo", "build", "--quiet", "-p", "d3d8", "--profile", "release-dll"]))
        stages.append(("contract", plugin, tool("check_contract.py", "--mge", mge)))
    the_set = round_set(commits, mwse_commit)
    folder = set_folder_of(the_set)
    if deploys:
        # The commits under test, not the HEADs at the time of the stage: a merge can move a HEAD during the round.
        commit_args = ["--plugin-commit", commits["plugin"], "--mge-commit", commits["mge"]] if commits else []
        stages.append(set_stage(SET_STAGE, plugin, the_set, label))
        if sites:
            stages.append((SITES_SET_STAGE, plugin, tool("check_mwse_sites.py", os.path.join(folder, "MWSE.dll"))))
        stages.append((DEPLOY_STAGE, plugin, tool("deploy_rest.py", "--set", folder, *commit_args)))
    if gate:
        gate_now = os.path.join(plugin, "target", "merge-step-now")
        on_list = ["--gate-list", GATE_LIST[0][0]] if GATE_LIST else []
        stages.append(("replay gate", plugin, tool("validate_retained.py", "baseline", "--mge-build", folder, "--baseline", baseline, "--now", gate_now, *on_list)))
        if touches(paths, ORDER_PATHS[kind]):
            stages.append((ORDER_STAGE, plugin, tool("validate_retained.py", "order", "--mge-build", folder, "--baseline", baseline, "--now", os.path.join(plugin, "target", "merge-step-order"))))
        under_test = commits or {}
        stages.append((HELPER_SOURCES_STAGE, plugin, [python, "-I", os.path.join(plugin, "tools", "merge_step.py"), HELPER_SOURCES,
                                                      os.path.join(maker_tree("render-link"), HELPER_DEP_INFO), HELPER_SOURCES_PATH,
                                                      maker_tree("nirender"), maker_tree("render-link"),
                                                      under_test.get("plugin", "unknown"), under_test.get("mge", "unknown"),
                                                      os.path.join(folder, "numidium.exe")]))
        if tips:
            # The base pair's set, or the set of an ancestor with equal parts when the pair has none (equal_parts_base).
            base_set = base_commits or round_set(tips, mwse_commit)
            stages.append(set_stage(BASE_STAGE, plugin, base_set, label))
            stages.append((INSTANCE_STAGE, plugin, tool("instance_ab.py", "run", "--stage", "grass", "--baseline", baseline, "--base-build", set_folder_of(base_set),
                                                        "--now-build", folder, "--out", os.path.join(plugin, "target", "merge-step-instance"))))
        if GATE_LIST:
            # W567 part D: the last stage, so it runs only when every stage before it passed (the stop at the first
            # failure): the listed pictures of the gate's run become the accepted ones, and the next round is equal.
            stages.append((ACCEPT_LIST_STAGE, plugin, tool("validate_retained.py", "accept-list", "--gate-list", GATE_LIST[0][0], "--mge-build", folder,
                                                           "--baseline", baseline, "--now", gate_now)))
    elif not deploys and touches(paths, DIST_PATHS[kind]):
        stages.append(("build of the dist", mge, ["cargo", "xtask", "dist"]))
    return stages


def require_tests(command):
    """Runs one command (cargo test) and echoes its output. Exit code of the command, or 1 when it passed and no test ran:
    a filter or a lost #[ignore] leaves such a stage with nothing to judge, and 0 tests are no pass."""
    done = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
    output = done.stdout.decode("utf-8", "replace")
    # What the console or the log file cannot encode becomes a "?", as the write would fail on it.
    encoding = sys.stdout.encoding or "utf-8"
    sys.stdout.write(output.encode(encoding, "replace").decode(encoding))
    sys.stdout.flush()
    ran = tests_ran(output)
    say("%d tests ran" % ran)
    if done.returncode == 0 and ran == 0:
        say("FAIL: no test ran, so the stage judged nothing")
        return 1
    return done.returncode


def needs_clean_sibling(stages):
    return any(name in ("replay gate", "d3d8 build", "build of the dist", SET_STAGE) for name, _, _ in stages)


def baseline_problem(stages, baseline):
    """Why the replay gate of these stages cannot compare, as a text, or None. main() asks before the take and the
    builds: a baseline with no captures failed the gate only after them (2026-11-09, Rob's run 8b51a23d59)."""
    if not any(name == "replay gate" for name, _, _ in stages) or has_captures(baseline):
        return None
    return "%s has no capture (*.nircap), so the replay gate cannot compare: give --baseline a folder with the captures" % baseline


def tree_problem(kind, repo, stages, head, commit, changed, sibling_changed=False):
    """Why this tree cannot take the stages, as a text, or None."""
    if kind is None:
        return "%s is no tree of %s or %s" % (repo, PLUGIN_NAME, MGE_NAME)
    if head != commit:
        return "the tree is at %s, not %s: merge first, as this tool makes no trial merge" % (head[:10], commit[:10])
    if changed:
        return "the tree has tracked changes"
    # The builds of the gate and the deploy are the maker's, in its own trees of the commits (W521): this tree's name does
    # not choose what they build, so a worktree with another name may run them.
    missing = [folder for _, folder, _ in stages if not os.path.isdir(folder)]
    if missing:
        return "no folder %s: the stages need the sibling" % missing[0]
    if sibling_changed and needs_clean_sibling(stages):
        return "the sibling %s has tracked changes: a build would hold them" % sibling_of(kind, repo)
    return None


def base_problem(base, commit, is_ancestor, tip_before, within_tip_before, last_passed=None, within_last_passed=True):
    """Why the diff from this base does not tell the change, as a text, or None. tip_before is <branch>@{1}, or None."""
    if base == commit:
        return "the base is the commit, so the diff is empty and asks for no stage: give the tip before the merge"
    if not is_ancestor:
        return "the base %s is no ancestor of the commit: the diff would also hold the other side's changes" % base[:10]
    if tip_before is None:
        return "no <branch>@{1}: the tree is on no branch, or the branch has no earlier tip, so nothing shows that the base holds the whole merge"
    if not within_tip_before:
        return "the base %s is newer than the tip before the last move of the branch, %s: the diff would skip a part of the merge" % (base[:10], tip_before[:10])
    if last_passed and not within_last_passed:
        return "the base %s is newer than the last commit that passed, %s: the diff would skip a merge that failed or had no run" % (base[:10], last_passed[:10])
    return None


def stage_env(environ, name=None, inputs=None):
    """The environment of the stage `name`: the shell's without BUILD_MOVERS, with cargo's folder first in PATH. The plugin's
    tests also get the switch NIRENDER_TESTS_NEED_INPUTS and their inputs (tests_inputs)."""
    env = {key: value for key, value in environ.items() if key not in BUILD_MOVERS}
    env["PATH"] = os.path.join(os.path.expanduser("~"), ".cargo", "bin") + os.pathsep + environ["PATH"]
    if name == TESTS_STAGE:
        env.update(tests_inputs() if inputs is None else inputs)
    return env


# The stage of the plugin's unit tests. In a round a test of it that lacks its input fails, in place of a skip (W527).
TESTS_STAGE = "plugin tests"
TESTS_NEED_INPUTS = "NIRENDER_TESTS_NEED_INPUTS"
# The inputs of those tests: the switch and the key of local.json that names each.
TESTS_INPUT_KEYS = (("NIRENDER_MCPATCH", "mcpatch_root"), ("NIRENDER_VANILLA_EXE", "vanilla_exe"))
TESTS_MWSE_DLL = "NIRENDER_MWSE_DLL"


def tests_inputs(read=None, deploys=None):
    """The variables of the plugin's tests in a round: the switch on, the places of local.json (the tree's, else the main
    checkout's), and the MWSE.dll of the first test install (the installs hold one MWSE, D107; the MWSE stages check that).
    A key that local.json does not have stays unset, and its test fails "NOT RUN in a round"."""
    read = read or (lambda key: local(key, None))
    deploys = MWSE_DEPLOYS if deploys is None else deploys
    env = {TESTS_NEED_INPUTS: "1"}
    for switch, key in TESTS_INPUT_KEYS:
        place = read(key)
        if place:
            env[switch] = place
    if deploys:
        env[TESTS_MWSE_DLL] = next(iter(deploys.values()))
    return env


def verdict(log, text):
    """One line of the round's verdict: on the console, and in the log, so that a kept log shows it."""
    say(text)
    log.write("== %s\n" % text)
    log.flush()


def cite_lines(kind, base, commit, tool=None, run=subprocess.run):
    """The lines of the cite check of the merge from `base` to `commit` (check_cites.py --merge, report only), each with
    "cites: " first. It never raises: a missing tool, a failing run or a timeout gives one line that says why the cites
    were not checked. A tree of no kind gives none."""
    repo_name = CITE_REPOS.get(kind)
    if not repo_name:
        return []
    tool = tool or docs_place(*CITE_TOOL_PARTS)
    if not os.path.isfile(tool):
        return ["cites: not checked: no %s" % tool]
    try:
        done = run([sys.executable, "-I", tool, "--merge", base, commit, "--repo", repo_name], capture_output=True, text=True,
                   timeout=CITE_TIMEOUT_SECONDS, env=git_env())
    except (OSError, subprocess.SubprocessError) as error:
        return ["cites: not checked: %s" % error]
    lines = [line for line in done.stdout.splitlines() if line.strip()]
    if done.returncode != 0:
        why = (lines or done.stderr.strip().splitlines() or ["no output"])[-1]
        return ["cites: not checked (exit %d): %s" % (done.returncode, why)]
    return ["cites: %s" % line for line in lines]


# The start of the last line of check_cites.py --merge, the frame map count of the close line (W528, D502).
FRAME_MAP = "frame map: "


def frame_map_of(cites):
    """The frame map part of a close line: the line of `cites` (cite_lines) that check_cites.py gives as
    "frame map: N cites moved, M steps touched". This tool does not count the cites itself. A cite check that did not
    run, or that gave no such line (an older check_cites.py), is "frame map: NOT RUN (<why>)": the docs pass takes the
    frame map as due then too."""
    for line in reversed(cites):
        text = line[len("cites: "):] if line.startswith("cites: ") else line
        if text.startswith(FRAME_MAP):
            return text
    why = next((line[len("cites: "):] for line in cites if line.startswith("cites: not checked")), "the cite check gave no frame map line")
    return "%sNOT RUN (%s)" % (FRAME_MAP, why)


# The start of the refusal of slot.py's `refuse_build_tool`, which replay.py calls for each replay of the gate.
SLOT_REFUSAL = "Refused by the slot check: "


def slot_refusal_in(output):
    """The first sentence of the last refusal of the slot check in a stage's output, or None. A replay that the slot
    check refused (no room on C, another holder) is no fault of the pictures, and the FAIL line must say so."""
    refusal = None
    for line in output.splitlines():
        at = line.find(SLOT_REFUSAL)
        if at >= 0:
            refusal = line[at + len(SLOT_REFUSAL):]
    if refusal is None:
        return None
    # The refusal names its tool first: "replay.py: only 29.0 GB free on C, ...".
    tool, colon, reason = refusal.partition(": ")
    if colon and (tool.endswith(".py") or tool == "build tool"):
        refusal = reason
    return refusal.split(". ")[0].strip()


def deploy_refusal_in(output):
    """The text of the last line 'REFUSED: <why>' in a stage's output (tools/deploy_rest.py: a game or Mod Organizer runs), or None."""
    refusal = None
    for line in output.splitlines():
        if line.startswith("REFUSED: "):
            refusal = line[len("REFUSED: "):].strip()
    return refusal


def output_since(log, start):
    """What the log file holds after the byte `start`: the output of the stage that wrote there."""
    with open(log.name, "rb") as file:
        file.seek(start)
        return file.read().decode("utf-8", "replace")


def not_run_reasons(output):
    """The reason of each "NOT RUN" line of a stage's output, in order."""
    return [line.partition(": ")[2] or line for line in output.splitlines() if line.startswith("NOT RUN")]


def not_run_reason(output):
    """The reason in the last "NOT RUN" line of a stage's output, or a plain text."""
    return (not_run_reasons(output) or ["the stage could not judge"])[-1]


def maker_missing_in(output):
    """The MISSING texts of the maker's probe ("probe: MISSING ..." lines of build_set.py) in a stage's output."""
    prefix = "probe: " + MISSING
    return [line[len(prefix):] for line in output.splitlines() if line.startswith(prefix)]


def mwse_withheld_reason(missing, command):
    """The reason of a set stage whose MWSE.dll lacks what the plugin needs (the maker's MISSING FEATURE, Q10): the round
    fails, and the line names the option that makes the round's set with another MWSE commit."""
    mwse = command[command.index("--mwse") + 1] if "--mwse" in command else "unknown"
    return ("MISSING %s in the MWSE.dll of MWSE %s; the stages that take the set do not run, and the round fails: run the "
            "round again with --mwse <an MWSE commit that has it>" % ("; ".join(missing) or "a feature", mwse[:10]))


def fallback_note(output):
    """The tail of a stage's verdict line that names the MWSE checks of its output that read the sibling MWSE (W497), or
    an empty text: a FAIL after such a fallback can be the sibling's branch, not a fault of the code."""
    names = fallback_checks(output)
    return "; MWSE FALLBACK in %s (the sibling MWSE was read, not the installs' commit)" % ", ".join(names) if names else ""


def format_change_verdict(name, seconds, log):
    """The verdict of a stage that exited with FORMAT_CHANGE_EXIT: the base refused the capture with a format code of the
    helper, and the build under test played it, so no A/B of the two builds is possible. The exit code decides; the
    stage's text is only shown. The stage passes only with an evidence file for it (--evidence), whose md5 goes into the
    verdict and the round's close line."""
    change = "FORMAT CHANGE (the base refuses the capture, the tip plays it: exit code %d of the stage)" % FORMAT_CHANGE_EXIT
    evidence = EVIDENCE.get(name)
    if evidence is None:
        verdict(log, "NOT RUN %s (%d s): %s; the round fails without an evidence file: --evidence \"%s=FILE\"" % (name, seconds, change, name))
        return noted(name, NOT_RUN_WORD, "a format change with no evidence file")
    path, md5 = evidence
    passed = "%s; evidence %s md5 %s" % (change, path, md5)
    FORMAT_CHANGES.append("%s: %s" % (name, passed))
    verdict(log, "PASS %s (%d s): NOT RUN: %s" % (name, seconds, passed))
    return noted(name, PASS_WORD)


def noted(name, word, reason=""):
    """Keeps the word of a stage's verdict in STAGE_WORDS. True when the word counts as passed."""
    STAGE_WORDS.append((name, word, reason))
    return word in (PASS_WORD, WITHHELD_WORD)


def evidence_of(values, exists=os.path.isfile, md5_of=None):
    """The evidence files of --evidence "STAGE=FILE" arguments, as {stage: (path, md5)}, or a problem as a text. Only a
    stage of EVIDENCE_STAGES takes one, once, and the file must exist."""
    md5_of = md5_of or file_md5
    found = {}
    for value in values or []:
        name, equals, path = value.partition("=")
        if not equals or not path:
            return "--evidence %s: give STAGE=FILE" % value
        if name not in EVIDENCE_STAGES:
            return "--evidence %s: only the stage %s takes an evidence file" % (value, " or ".join(EVIDENCE_STAGES))
        if name in found:
            return "--evidence %s: a second file for the stage %s" % (value, name)
        if not exists(path):
            return "--evidence %s: no such file" % value
        found[name] = (os.path.abspath(path), md5_of(path))
    return found


def file_md5(path):
    with open(path, "rb") as file:
        return hashlib.md5(file.read()).hexdigest()


def run_stage(name, folder, command, log):
    began = time.time()
    log.write("\n== %s: %s in %s\n" % (name, " ".join(command), folder))
    log.flush()
    start = log.tell()
    code = subprocess.run(command, cwd=folder, env=stage_env(os.environ, name), stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL).returncode
    failure = ""
    if code == NOT_RUN_CODE:
        output = output_since(log, start)
        reason = not_run_reason(output)
        verdict(log, "NOT RUN %s (%d s): %s" % (name, time.time() - began, reason))
        return noted(name, NOT_RUN_WORD, reason)
    if code == FORMAT_CHANGE_EXIT and name in EVIDENCE_STAGES:
        return format_change_verdict(name, time.time() - began, log)
    if code != 0:
        output = output_since(log, start)
        if name in (SET_STAGE, BASE_STAGE) and code == MISSING_FEATURE_EXIT:
            # The code under test may be right; the MWSE.dll of the round's MWSE commit is too old for it, and the maker made
            # no set (G1). The stages that take the set are NOT RUN (run_stages), so the round fails (Q10).
            reason = mwse_withheld_reason(maker_missing_in(output), command)
            verdict(log, "%s %s (%d s): %s" % (WITHHELD_WORD, name, time.time() - began, reason))
            return noted(name, WITHHELD_WORD, reason)
        refusal, deploy_refusal = slot_refusal_in(output), deploy_refusal_in(output)
        if name == DEPLOY_STAGE and code == DEPLOY_REFUSED_CODE and deploy_refusal:
            # A game or Mod Organizer runs: nothing was written. The merge is no worse for it, so the stage does not fail the
            # round (D193); the line names the command that deploys the same build later, when nothing runs.
            # Quoted as the Windows command line takes it, so that a path with a space pastes into a shell.
            not_deployed = "%s %s (%d s): %s. To deploy this build alone, run in %s: %s" % (
                WITHHELD_WORD, name, time.time() - began, deploy_refusal, subprocess.list2cmdline([folder]), subprocess.list2cmdline(command))
            verdict(log, not_deployed)
            NOT_DEPLOYED.append(not_deployed)
            return noted(name, WITHHELD_WORD, deploy_refusal)
        failure = ": exit code %d%s" % (code, "; refused by the slot check: %s" % refusal if refusal else "; refused: %s" % deploy_refusal if deploy_refusal else "")
    failure += fallback_note(output_since(log, start))
    word = PASS_WORD if code == 0 else FAIL_WORD
    verdict(log, "%s %s (%d s)%s" % (word, name, time.time() - began, failure))
    return noted(name, word, failure.lstrip(": "))


def log_path_of(commit, folder=LOG_FOLDER, prefix="merge-step"):
    """The file of the output of a round, in a folder that exists. A round never writes into the log of an earlier one: the
    first round of a commit has `merge-step-<commit>.log`, the next ones `merge-step-<commit>-2.log` and so on (2026-11-09:
    a rerun overwrote the log of a refused round). The file is made here, empty, so that two rounds cannot take one name.
    A --quick round has the prefix `quick`."""
    os.makedirs(folder, exist_ok=True)
    for number in itertools.count(1):
        path = os.path.join(folder, "%s-%s%s.log" % (prefix, commit[:10], "" if number == 1 else "-%d" % number))
        try:
            with open(path, "x", encoding="utf-8"):
                return path
        except FileExistsError:
            continue


def clear_pictures(folder):
    """Removes the files of the gate's folder of pictures, of its tour folder and of the order stage's folder, one by
    one."""
    for part in (folder, os.path.join(folder, "tour"), os.path.join(folder, "order")):
        if os.path.isdir(part):
            for entry in os.scandir(part):
                if entry.is_file(follow_symlinks=False):
                    os.remove(entry.path)


def run_stages(stages, log, note_stage=lambda: None):
    """The stages in their order, up to the first one that does not pass (W567 part B): each stage after it is NOT RUN,
    with the name of that stage, and the round fails. A WITHHELD stage counts as passed, so the round goes on, but the
    stages that take a set that was not made are NOT RUN. The fast stages come first (`fast_stages`), so a fault of the
    code stops the round before the set, the deploy and the gate.
    `note_stage` runs before each stage that runs: the hold of the build slot lapses after 15 minutes with
    no compile seen, and a round can pass that with a stage that does not compile. It returns False when the holds could
    not be taken again: the stage is then NOT RUN, and counts as not passed."""
    results = {}
    # Whether each set stage made its set: only a PASS did (a WITHHELD set stage made none).
    made = {}
    # The first stage that did not pass: it stops the round.
    stopped_by = None
    del NOT_DEPLOYED[:]
    del FORMAT_CHANGES[:]
    del STAGE_WORDS[:]

    def not_run(name, reason):
        verdict(log, "%s %s: %s" % (NOT_RUN_WORD, name, reason))
        return noted(name, NOT_RUN_WORD, reason)

    for name, folder, command in stages:
        if stopped_by is not None:
            results[name] = not_run(name, "%s did not pass, and the round stops at its first failure" % stopped_by)
            continue
        if name in (SITES_SET_STAGE, DEPLOY_STAGE, "replay gate", ORDER_STAGE, HELPER_SOURCES_STAGE, INSTANCE_STAGE, ACCEPT_LIST_STAGE) and not made.get(SET_STAGE, True):
            results[name] = not_run(name, "the round's set was not made")
            stopped_by = name
            continue
        if name == INSTANCE_STAGE and not made.get(BASE_STAGE, True):
            results[name] = not_run(name, "the base set was not made")
            stopped_by = name
            continue
        if name in ("replay gate", ORDER_STAGE):
            clear_pictures(command[command.index("--now") + 1])
        if note_stage() is False:
            results[name] = not_run(name, "the slot hold of this run ended and was not taken again")
            stopped_by = name
            continue
        results[name] = run_stage(name, folder, command, log)
        if name in (SET_STAGE, BASE_STAGE):
            made[name] = STAGE_WORDS[-1][1] == PASS_WORD
        if not results[name]:
            stopped_by = name
    return list(results.values())


def stages_column(words):
    """The stages column of a close line (W528): the count of each word, the names of the stages that passed, and each
    WITHHELD stage with its reason. `words` are (stage, word, reason), as STAGE_WORDS holds them."""
    counts = ["%s %d" % (word, count) for word in STAGE_WORD_ORDER for count in [sum(1 for _, said, _ in words if said == word)] if count]
    passed = [name for name, word, _ in words if word == PASS_WORD]
    column = ["%s of %d: %s" % (", ".join(counts) or "no stage", len(words), ", ".join(passed) or "none")]
    column += ["%s %s: %s" % (word, name, reason) for name, word, reason in words if word != PASS_WORD]
    return " ".join("; ".join(column).split("\t"))


def summary_line(results, log_path, words=None):
    """The last line of a round: the stages that passed and those WITHHELD, apart, of all (W528, verdict 467: a WITHHELD
    stage counts as passed, but it deployed nothing). `words` are STAGE_WORDS by default; when their count is not the
    count of the results, the line gives the passes alone."""
    words = STAGE_WORDS if words is None else words
    if len(words) != len(results):
        return "%d of %d stages passed; the output is in %s" % (sum(results), len(results), log_path)
    said = [word for _, word, _ in words]
    return "%d passed, %d withheld of %d stages; the output is in %s" % (
        said.count(PASS_WORD), said.count(WITHHELD_WORD), len(results), log_path)


def moves_the_ref(baseline):
    """Only a pass against the accepted folder is a pass of the merge."""
    return os.path.normcase(os.path.abspath(baseline)) == os.path.normcase(os.path.abspath(BASELINE))


def items_of(subject):
    """The item that a merge subject names at its end, as a list of one, or no item. The item of a part keeps the part
    ("W341 step A"); a row closes whole only from a subject without a part ("W341"). A tail of decisions is dropped
    ("(W363, D325)" is W363)."""
    match = ITEM.search(subject.strip())
    return [" ".join(match.group(1).split())] if match else []


def unread_item_lines(merges):
    """One WARNING line for each (commit, subject) of `merges` whose last parentheses name a W number but no item that
    items_of reads: such a merge closes nothing, and the round must say so (W528: bdc4145 "(W527, W524 row 2)" closed
    nothing, and the round said only "nothing to write")."""
    lines = []
    for commit, subject in merges:
        last = LAST_PARENTHESES.search(subject)
        if last and re.search(r"\bW\d+", last.group(1)) and not items_of(subject):
            lines.append("WARNING: %s closes nothing: its subject ends in \"(%s)\", which names no item. The forms: (W<n>) or "
                         "(W<n> <part>), then any tail of D<n>, D<n>/D<m>, W<n> <part>, risk <n> or E<n>, each after \", \". "
                         "The subject: %s" % (commit[:10], last.group(1), subject))
    return lines


def closing_commits(entries, tip):
    """The commits of a check that can close an item, oldest first: each merge on the first-parent path, and the checked
    commit itself. `entries` are (commit, parents, subject) of that path, oldest first, as `first_parent_path` gives them.
    A batch of merges with one check (the owner's D183) closes each merge's item, not only the tip's. A commit that is no
    merge closes nothing unless it is the checked commit, and a commit of a merged branch is not on the path."""
    return [(commit, subject) for commit, parents, subject in entries if len(parents.split()) > 1 or commit == tip]


def first_parent_path(repo, base, commit):
    """(commit, parents, subject) of each commit on the first-parent path from `base` (left out) to `commit`, oldest first."""
    out = git(repo, "log", "--first-parent", "--reverse", "--format=%H%x09%P%x09%s", "%s..%s" % (base, commit))
    return [tuple(line.split("\t", 2)) for line in out.splitlines() if line.count("\t") >= 2]


def fast_forward_warning(entries, tip):
    """A warning line when the first-parent path holds no merge commit with an item and the checked commit is no merge, or
    None. A fast-forward of a batch leaves no merge commit, and then only the tip's item closes (the Coordinator merges
    with --no-ff, D183)."""
    def is_merge(parents):
        return len(parents.split()) > 1

    if any(is_merge(parents) and items_of(subject) for _, parents, subject in entries):
        return None
    if any(commit == tip and is_merge(parents) for commit, parents, _ in entries):
        return None
    return ("WARNING: base..%s holds no merge commit with a (W<n>) subject, and %s is no merge: a fast-forward closes only "
            "the item of the checked commit (merge with --no-ff)" % (tip[:10], tip[:10]))


def closing_merges(repo, base, commit):
    """The (commit, subject) of a check of base..commit that can close an item, oldest first, and the fast-forward warning
    of that path (or None)."""
    entries = first_parent_path(repo, base, commit)
    return closing_commits(entries, commit), fast_forward_warning(entries, commit)


def close_lines(existing, when, repo_name, merges, stages, log_path):
    """The closing lines of a pass: one for each (commit, subject) of `merges` whose subject names an item that the lines
    `existing` of the closes file do not have for that commit. A rerun of a commit adds nothing; a later commit of the same
    item (its part b) adds a line."""
    have = set()
    for line in existing:
        columns = line.rstrip("\r\n").split("\t")
        if len(columns) >= 4:
            have.add((columns[1], columns[3]))
    lines = []
    for commit, subject in merges:
        commit, subject = commit[:10], subject.replace("\t", " ")
        for item in items_of(subject):
            if (item, commit) not in have:
                have.add((item, commit))
                lines.append("\t".join((when, item, repo_name, commit, stages, log_path, subject)))
    return lines


def write_closes(path, when, repo_name, merges, stages, log_path, tries=CLOSE_TRIES, wait=CLOSE_WAIT_SECONDS):
    """Appends the closing lines of a pass to `path` (CRLF, ASCII), with the header first when the file is new. The open
    is a plain append, which leaves the file to its readers. A reader that holds the file makes a PermissionError: the
    append is tried again, then given up with a line, as a missing close must not fail the check. The lines written, or
    None when the append was given up."""
    for attempt in range(tries):
        try:
            existing = []
            if os.path.exists(path):
                with open(path, encoding="ascii", errors="replace") as closes:
                    existing = closes.readlines()
            lines = close_lines(existing, when, repo_name, merges, stages, log_path)
            if lines:
                with open(path, "a", encoding="ascii", errors="replace", newline="\r\n") as closes:
                    if not existing:
                        closes.write(CLOSES_HEADER + "\n")
                    closes.write("".join(line + "\n" for line in lines))
            return lines
        except PermissionError as error:
            if attempt + 1 == tries:
                say("closes: NOT WRITTEN to %s after %d tries (%s); the check stands, write the line by hand" % (path, tries, error))
                return None
            time.sleep(wait)


def close_after(results, baseline, merges, write, words=None, extra=()):
    """The closing lines of a round, through `write(merges, stages)`: only when every stage passed and the pass is
    against the accepted folder, as only that is a pass of the merge. The stages column names each stage by its word
    (stages_column of `words`), and each FORMAT CHANGE verdict with the md5 of its evidence file. None when the round
    writes no close."""
    if not results or not all(results) or not moves_the_ref(baseline):
        return None
    words = STAGE_WORDS if words is None else words
    if len(words) != len(results):
        column = "%d of %d (the words of the stages are not known: %d words)" % (sum(results), len(results), len(words))
    else:
        column = stages_column(words)
    return write(merges, "; ".join([column] + FORMAT_CHANGES + list(extra)))


def deploys_open(plugin_repo, mge_repo, path=DEPLOYS_FILE):
    """The open deploys of the deploys file (common.open_deploys): a commit is newer when it is the same or a descendant
    in its repo. No file: none open."""
    if not os.path.isfile(path):
        return []
    with open(path, encoding="ascii", errors="replace") as file:
        lines = file.readlines()
    repos = {"plugin": plugin_repo, "mge": mge_repo}

    def is_newer(kind, older, newer):
        repo = repos[kind]
        return bool(repo) and resolve(repo, older) is not None and resolve(repo, newer) is not None and is_ancestor(repo, older, newer)

    return open_deploys(lines, is_newer)


def open_deploy_lines(still):
    """The open deploys as lines: the machine answer, then one line for each."""
    return [open_deploys_text(still)] + ["  %s: plugin %s, mge %s: %s" % (when, plugin[:10], mge[:10], what) for when, plugin, mge, what in still]


def withhold_deploy(path, when, commits, lines):
    """Appends the WITHHELD line of a round that passed with its deploy kept back. A failed write is a line, not a
    failure of the round."""
    plugin, mge = (commits or {}).get("plugin"), (commits or {}).get("mge")
    try:
        append_deploys(path, deploys_line(when, DEPLOY_WITHHELD, plugin, mge, " | ".join(lines)))
        return "deploys: WITHHELD plugin %s, mge %s in %s" % ((plugin or "unknown")[:10], (mge or "unknown")[:10], path)
    except OSError as error:
        return "deploys: NOT WRITTEN to %s (%s); write the WITHHELD line by hand" % (path, error)


def resolve(repo, name):
    """The commit of a name, or None. A name of a tree or a blob is no commit; a tag gives the commit it points to."""
    return subprocess.run(["git", "-C", repo, "rev-parse", "--verify", "-q", name + "^{commit}"], capture_output=True, text=True, env=git_env()).stdout.strip() or None


def branch_of(repo):
    """The branch of the tree, or None."""
    return subprocess.run(["git", "-C", repo, "symbolic-ref", "-q", "--short", "HEAD"], capture_output=True, text=True, env=git_env()).stdout.strip() or None


def is_ancestor(repo, older, newer):
    return subprocess.run(["git", "-C", repo, "merge-base", "--is-ancestor", older, newer], env=git_env()).returncode == 0


# The stages that need the game slot (the owner's D183): the replay gate replays on the GPU, and the instance A/B replays
# a capture of the game with three builds, and the deploy writes its set f<N> to E: and refuses while a game runs (W406:
# a round can deploy with no gate). A new stage that starts the game or times the GPU belongs here. A check with
# none of them leaves the game slot to the game runs; before D183 every check held it (D161), and the workers waited.
GAME_STAGES = ("replay gate", ORDER_STAGE, INSTANCE_STAGE, DEPLOY_STAGE)


def needs_game(stages):
    return any(name in GAME_STAGES for name, _, _ in stages)


# A path on the evidence disk E:, as a whole argument or after "=" (--out=E:/x): E: with any rest (E:x is relative to the
# folder of E:), or /e/ of Git Bash.
EVIDENCE_PATH = re.compile(r"(?i)(?:^|=)(?:e:|/e/)")


def evidence_problem(stages):
    """Why these stages may not run as one round, as a text, or None. A round with no stage of GAME_STAGES leaves the game
    slot free, so a game can run beside it, and none of its stages may then write to E: (the Coordinator's D183): a base
    build or a copy to E:\\nirender-keep belongs in a round that holds the game slot. A stage counts as a writer when its
    folder or an argument of its command is a path on E:. The log and the closes file of this tool are small appends,
    and they stay allowed."""
    if needs_game(stages):
        return None
    for name, folder, command in stages:
        on_e = [text for text in (folder, *command) if EVIDENCE_PATH.search(str(text))]
        if on_e:
            return "the stage %r names %s on E:, and no stage of the round holds the game slot (%s)" % (name, on_e[0], ", ".join(GAME_STAGES))
    return None


# The refusal of slot.py when this session already holds the build slot under another label: another run of this
# session, usually an earlier merge_step that still runs. The first form is slot.py's until 2026-11-09, the second its
# plainer form after it.
OWN_HOLD = ("this session holds it as", "your session already holds the build slot as")


def take_options(slots, options):
    """The keywords of a take: `options`, and slot.py's --another (W361) when this slot.py has it, so that the place of
    this run does not replace another place of its session in the queue."""
    another = getattr(slots, "ANOTHER", None)
    return {**(options or {}), **({another: True} if another else {})}


def take_slot(slots, slot, what, no_wait, options=None, minutes=SLOT_MINUTES):
    """Takes a slot ("build", "game", or "both": the game and the whole build side together), and asks again in its
    queue, so that the place stays. A hold of another run of this session (an earlier merge_step) cannot overlap this run:
    it waits for that release too. False, with a "NOT RUN" line, with --no-wait when it must wait, on the end of the wait
    after a `leave` of its place, and on any other refusal, as waiting cannot pass that. A Ctrl-C in the wait leaves the
    queue."""
    os.makedirs(slots.STATE_DIR, exist_ok=True)
    waiting_for_own = False
    with slot_asker(slots, slot) as ask:
        while True:
            try:
                reply = ask(what, minutes=minutes, **take_options(slots, options))
            except slots.StateBusy as error:
                say("the state of the slots is busy, and I ask again: %s" % error)
                time.sleep(WAIT_SECONDS)
                continue
            except KeyboardInterrupt:
                slots.leave(slot, what)
                raise
            if reply.code == slots.DONE:
                # All lines: a take of the build slot names the free space on C, and warns when the cleaning is due.
                say(reply.text)
                return True
            first = reply.text.splitlines()[0]
            if reply.code == getattr(slots, "WAIT_ENDED", None):
                say("NOT RUN: this run left the queue of the %s slot: %s" % (slot, first))
                return False
            own = reply.code == slots.REFUSED and any(phrase in first for phrase in OWN_HOLD)
            if reply.code == slots.REFUSED and not own:
                say("NOT RUN: the take of the %s slot was refused: %s" % (slot, first))
                return False
            if no_wait:
                if not own:
                    slots.leave(slot, what)
                say("NOT RUN: --no-wait, and the %s slot is not free: %s" % (slot, first))
                return False
            if own and not waiting_for_own:
                say("waiting: another run of this session holds the %s slot, and this run starts after its release: %s" % (slot, first))
            elif not own:
                say(first)
            waiting_for_own = own
            time.sleep(WAIT_SECONDS)


def take_build(slots, what, no_wait, options=None):
    return take_slot(slots, "build", what, no_wait, options)


# The slots of a take of both (slot.py, D208).
SLOTS_OF_BOTH = ("game", "build")


def round_slot(stages):
    """The take of a round: "both" (the game and the whole build side in one take) when a stage needs the game, else
    "build". A round that needs both never holds one while it waits for the other (the owner's D208; the deadlock of
    14:37 on 2026-11-10: this tool held the build side and waited for the game, and a quiet window the other way)."""
    return "both" if needs_game(stages) else "build"


def release_slot(slots, slot, what):
    """Releases the round's hold by its label and prints every line of the reply: after a build, slot.py names the free
    space on C and warns when the cleaning is due (the owner's D138 and D141). From a terminal with no session, slot.py
    owns a hold only by its label."""
    say(slots.release(slot, what).text)


@contextlib.contextmanager
def game_hold_noted(slots, seconds=GAME_NOTE_SECONDS, build_who=None):
    """Notes the work of the game slot every `seconds` while the stages run: its hold lapses after 15 minutes with no game
    seen, and the stages start none. With `build_who`, the label of a take, it also notes the build work of the places of
    that take (make_play_set.py, W405: a live run compiles nothing, and its hold of the build side would lapse). A run that
    dies stops the notes, and its hold lapses by itself."""
    stop = threading.Event()

    def note():
        while True:
            now = time.time()
            slots.note_active("game", now)
            if build_who is not None:
                slots.note_holder_build(slots.owner_of(slots.session_id(), build_who), now)
            if stop.wait(seconds):
                return

    thread = threading.Thread(target=note, daemon=True)
    thread.start()
    try:
        yield
    finally:
        stop.set()
        thread.join()


def places_held(slots, slot, who):
    """How many places of a slot the run `who` holds, or None when the state cannot be read now (a hold is then taken as
    kept: the next stage asks again). The build side has two places; the game slot has one."""
    read_record = getattr(slots, "read_record", None)
    if read_record is None:
        return None
    owner = slots.owner_of(slots.session_id(), who)
    count = 0
    for unit in slots.units_of(slot):
        try:
            holder = read_record(slots.holder_path(unit), slots.Holder)
        except slots.StateError:
            return None
        count += holder is not None and holder.session == owner and holder.who == who
    return count


def keep_holds(slots, what, slot, log):
    """Before a stage: the run still holds the slots of its take (`round_slot`; "game" for the last stages of a play set,
    make_play_set.py), and takes again one that lapsed when it is free at once. A wait for it while the run holds the other slot is a hold and wait (D208): slot.py refuses it, and
    the stage is then NOT RUN. The notes of the work (`game_hold_noted`, and here the build work) keep the holds while the
    stages run. False when a hold is lost."""
    owner = slots.owner_of(slots.session_id(), what)
    # The build work of the places of this run, as the hook and the tool checks of slot.py note it: a hold of another run
    # gets no note.
    slots.note_holder_build(owner, time.time())
    wanted = {"both": [("game", 1), ("build", len(slots.units_of("build")))], "game": [("game", 1)]}.get(slot, [("build", 1)])
    for part, needed in wanted:
        held = places_held(slots, part, what)
        if held is None or held >= needed:
            continue
        verdict(log, "the %s hold of this run ended (it lapsed; %d of %d place(s) held); taking it again if it is free" % (part, held, needed))
        options = {"whole": True} if part == "build" and needed > 1 else {}
        if not take_slot(slots, part, what, True, options, minutes=GAME_SLOT_MINUTES if part == "game" else SLOT_MINUTES):
            return False
        verdict(log, "re-took the %s at %s" % ("game slot" if part == "game" else "build side", time.strftime("%H:%M")))
    return True


def run_round(slots, stages, what, log_path, no_wait):
    """The stages of one round with the slots of `round_slot` held, released on every exit path: a failing stage, an
    exception and Ctrl-C. (results, summary), or None when the take did not give the slots."""
    problem = evidence_problem(stages)
    if problem:
        raise ValueError("merge_step refuses the round: %s" % problem)
    slot = round_slot(stages)
    if slot == "build":
        say("the stages need no game slot: the game stays free for the game runs (D183)")
    if not take_slot(slots, slot, what, no_wait, minutes=GAME_SLOT_MINUTES if slot == "both" else SLOT_MINUTES):
        return None
    try:
        with game_hold_noted(slots) if slot == "both" else contextlib.nullcontext(), open(log_path, "w", encoding="utf-8", errors="replace") as log:
            results = run_stages(stages, log, lambda: keep_holds(slots, what, slot, log))
            # In the log before it closes: the last line of a kept log is the round's verdict. The console gets it after the slot's line.
            summary = summary_line(results, log_path)
            log.write("\n== %s\n" % summary)
    finally:
        release_slot(slots, slot, what)
    return results, summary


def quick_plan(kind, tree, sibling, base):
    """The stages of a --quick round of `tree` (fast_stages), as (stages, None), or (None, why) when they cannot run. A
    plugin tree takes `sibling` for its docs when that is a render-link tree; a render-link tree needs a plugin tree there,
    as the plugin's tools run its stages."""
    if kind is None:
        return None, "%s is no tree of %s or %s" % (tree, PLUGIN_NAME, MGE_NAME)
    sibling_kind = repo_kind(sibling) if sibling and os.path.isdir(sibling) else None
    if kind == "plugin":
        return fast_stages(kind, tree, sibling if sibling_kind == "mge" else None, base), None
    if sibling_kind != "plugin":
        return None, "the stages of render-link run the tools of a %s tree, and %s is none: give --sibling <a %s tree>" % (
            PLUGIN_NAME, sibling, PLUGIN_NAME)
    return fast_stages(kind, sibling, tree, base), None


def quick_head(repo_name, commit, sibling=None):
    """The start of a --quick line: the repo and its commit, and the sibling tree that the stages read with its commit
    (`sibling`: (name, commit)), or "no sibling"."""
    return "%s %s (%s)" % (repo_name, commit[:10], "sibling %s %s" % (sibling[0], sibling[1][:10]) if sibling else "no sibling")


def quick_line(head, words, log_path):
    """The one line of a --quick round for the done report, from `words` (STAGE_WORDS: stage, word, reason); `head` is
    quick_head's. It passes only when every stage passed; else it names the first stage that did not."""
    failed = next(((name, word, reason) for name, word, reason in words if word != PASS_WORD), None)
    if words and failed is None:
        return "QUICK PASS %s: %d of %d (%s); log %s" % (head, len(words), len(words), ", ".join(name for name, _, _ in words), log_path)
    if failed is None:
        return "QUICK FAIL %s: no stage ran; log %s" % (head, log_path)
    name, word, reason = failed
    return "QUICK FAIL %s: %s (%s%s); log %s" % (head, name, word, ": " + reason if reason else "", log_path)


def run_quick(args):
    """A --quick round (W567 part A): the fast stages of the worker's tree at its HEAD, in one build place, and its line.
    Exit code 0 on a pass, 1 on a failure or a refusal, 3 when --no-wait finds no free place."""
    tree = os.path.abspath(args.repo or os.getcwd())
    kind = repo_kind(tree)
    repo_name = {"plugin": PLUGIN_NAME, "mge": MGE_NAME}.get(kind, os.path.basename(tree))
    if kind is None:
        say("QUICK FAIL %s: %s is no tree of %s or %s" % (repo_name, tree, PLUGIN_NAME, MGE_NAME))
        return 1
    commit = git(tree, "rev-parse", "HEAD")
    if git(tree, "status", "--porcelain", "--untracked-files=no"):
        say("QUICK FAIL %s %s: the tree has tracked changes, and the line names a commit: commit them first" % (repo_name, commit[:10]))
        return 1
    if args.base:
        base = resolve(tree, args.base)
        if base is None:
            say("QUICK FAIL %s %s: --base %s is no commit" % (repo_name, commit[:10], args.base))
            return 1
    else:
        shared = resolve(tree, SHARED_BRANCHES[kind])
        if shared is None:
            say("QUICK FAIL %s %s: no branch %s to take the base from: give --base <commit>" % (repo_name, commit[:10], SHARED_BRANCHES[kind]))
            return 1
        base = git(tree, "merge-base", "HEAD", shared)
    sibling = os.path.abspath(args.sibling) if args.sibling else sibling_of(kind, tree)
    stages, why = quick_plan(kind, tree, sibling, base)
    if why:
        say("QUICK FAIL %s %s: %s" % (repo_name, commit[:10], why))
        return 1
    # The stages read the sibling's working tree (the docs build the helper there; a render-link round runs the plugin's
    # tools): the line names its commit, and a sibling with tracked changes is refused, as the tree is.
    sibling_used = kind == "mge" or (os.path.isdir(sibling) and repo_kind(sibling) == "mge")
    sibling_head = None
    if sibling_used:
        sibling_head = (os.path.basename(sibling), git(sibling, "rev-parse", "HEAD"))
        if git(sibling, "status", "--porcelain", "--untracked-files=no"):
            say("QUICK FAIL %s: the sibling %s has tracked changes, and its stages read its working tree: commit them first, or give --sibling "
                "another tree" % (quick_head(repo_name, commit, sibling_head), sibling))
            return 1
    head = quick_head(repo_name, commit, sibling_head)
    say("quick: %s, base %s, sibling %s; stages: %s" % (head, base[:10], sibling if sibling_used else "none (the docs skip the helper)",
                                                      ", ".join(name for name, _, _ in stages)))
    if args.dry_run:
        return 0
    slots = load_slots()
    log_path = log_path_of(commit, os.path.join(tree, "target"), "quick")
    done = run_round(slots, stages, QUICK_LABEL % (repo_name, commit[:10]), log_path, args.no_wait)
    if done is None:
        return 3
    results, summary = done
    say(summary)
    line = quick_line(head, STAGE_WORDS, log_path)
    with open(log_path, "a", encoding="utf-8") as log:
        log.write(line + "\n")
    say(line)
    return 0 if results and all(results) else 1


def git(repo, *args):
    done = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, env=git_env())
    if done.returncode != 0:
        raise SystemExit("git %s in %s: %s" % (" ".join(args), repo, done.stderr.strip()))
    return done.stdout.strip()


def changed_paths(repo, older, newer):
    """The paths that the diff from `older` to `newer` changes. A moved file gives its old and its new path: a move out
    of a gate path changes the gate's input (W528)."""
    return git(repo, "diff", "--name-only", "--no-renames", older, newer).splitlines()


PAIR_REFS = "refs/merge-step-pair"


def pair_ref(kind, commit):
    """The ref that keeps the sibling's commit of the last pass of `commit`, a commit of the repo of `kind`: the pair that
    passed. It is a ref of the sibling's repo, as only that repo has the commit it names."""
    return "%s/%s/%s" % (PAIR_REFS, kind, commit)


def keep_pair(kind, sibling, commit, sibling_commit):
    """Keeps the pair of a pass: pair_ref(kind, commit) in the sibling's repo names `sibling_commit`."""
    git(sibling, "update-ref", pair_ref(kind, commit), sibling_commit)


def ref_say(log_path, text):
    """A line about a ref: on the console, and after the summary in the round's log (W532), so that the log names what
    its own round did to each ref."""
    say(text)
    if log_path:
        with open(log_path, "a", encoding="utf-8") as log:
            log.write(text + "\n")


def keep_pass(kind, repo, passed_ref, commit, sibling, sibling_head, log_path=None):
    """Moves refs/merge-step/<branch> to the commit, and keeps the pair of the round in the sibling: the base pair of the
    next check (tips_for). No pair when the sibling's HEAD moved during the round. Each line also goes to `log_path`."""
    old = resolve(repo, passed_ref)
    git(repo, "update-ref", passed_ref, commit)
    name = os.path.basename(repo)
    if old is None:
        ref_say(log_path, "%s: %s is set to %s: this round passed" % (name, passed_ref, commit[:10]))
    elif old == commit:
        ref_say(log_path, "%s: %s is at %s already: this round passed" % (name, passed_ref, commit[:10]))
    else:
        ref_say(log_path, "%s: %s moves from %s to %s: this round passed" % (name, passed_ref, old[:10], commit[:10]))
    if sibling_head and resolve(sibling, "HEAD") == sibling_head:
        keep_pair(kind, sibling, commit, sibling_head)
        ref_say(log_path, "this round's pair: %s in the sibling %s: %s" % (pair_ref(kind, commit), os.path.basename(sibling), sibling_head[:10]))
    elif sibling_head:
        ref_say(log_path, "this round's pair: none kept: the HEAD of the sibling moved during the round (%s at the start)" % sibling_head[:10])


def settle_refs(kind, repo, passed_ref, commit, sibling, sibling_head, stages, results, baseline, commits,
                sibling_changed, log_path, mwse_commit=None):
    """The refs after a round, and the round's exit code: a pass against the accepted folder moves the round's ref and
    can move the sibling's; any other end keeps both, with a line. Every line also goes to the round's log."""
    old = resolve(repo, passed_ref)
    stays = "%s: %s stays at %s" % (os.path.basename(repo), passed_ref, old[:10] if old else "nothing")
    if not all(results):
        ref_say(log_path, "%s: a stage did not pass" % stays)
        return 1
    if not moves_the_ref(baseline):
        ref_say(log_path, "%s: this pass is against %s, not the accepted folder" % (stays, baseline))
        return 0
    keep_pass(kind, repo, passed_ref, commit, sibling, sibling_head, log_path)
    if os.path.isdir(sibling):
        move_sibling_ref(kind, sibling, sibling_head, stages, results, baseline, commits, sibling_changed, log_path, mwse_commit)
    return 0


def tips_for(kind, repo, base, sibling_base=None):
    """The commits of the base set: `base` for the tree's repo, and for the sibling the commit that passed with `base`,
    which the pass kept in pair_ref(kind, base). The sibling's HEAD is not the base: it can hold code that `base` lacks
    (2026-11-10, a render-link tip that needs W84 with the plugin's last pass: "no field `like_exterior`"; 2026-11-09,
    nirender 9f68cd1 with a render-link that calls the spot cone). `sibling_base` names the sibling's commit in place of the
    kept one, for a base with no kept pair. The sibling's commit must be an ancestor of the sibling's HEAD (or HEAD
    itself). (tips, None), or (None, why not)."""
    other = "mge" if kind == "plugin" else "plugin"
    sibling = sibling_of(kind, repo)
    if not os.path.isdir(sibling):
        return None, "no sibling %s for the base set" % sibling
    sibling_head = resolve(sibling, "HEAD")
    if sibling_head is None:
        return None, "the sibling %s has no HEAD for the base set" % sibling
    if sibling_base is None:
        source = "the kept pair %s" % pair_ref(kind, base)
        sibling_commit = resolve(sibling, pair_ref(kind, base))
        if sibling_commit is None:
            return None, "the sibling %s keeps no pair for the base %s (%s): give --sibling-base, the sibling's commit that passed with the base" % (sibling, base[:10], pair_ref(kind, base))
    else:
        source = "--sibling-base %s" % sibling_base
        sibling_commit = resolve(sibling, sibling_base)
        if sibling_commit is None:
            return None, "--sibling-base %s is no commit of the sibling %s" % (sibling_base, sibling)
    if not is_ancestor(sibling, sibling_commit, sibling_head):
        return None, "%s is not an ancestor of the HEAD of the sibling %s (%s)" % (source, sibling, sibling_head[:10])
    return {kind: base, other: sibling_commit}, None


# The files of a cargo dep-info file: its home is build_set.py, whose drift test of the part inputs reads them too.
dep_info_paths = build_set.dep_info_paths


def sources_by_repo(paths, roots):
    """{kind: the sorted paths of `paths` under the folder roots[kind], relative to it with "/"}. A path under no root,
    or under a part of NOT_SOURCES, is left out."""
    found = {kind: set() for kind in roots}
    for path in paths:
        full = os.path.abspath(path)
        for kind, root in roots.items():
            top = os.path.abspath(root) + os.sep
            if os.path.normcase(full).startswith(os.path.normcase(top)):
                relative = full[len(top):].replace(os.sep, "/")
                if not relative.startswith(NOT_SOURCES):
                    found[kind].add(relative)
                break
    return {kind: sorted(names) for kind, names in found.items()}


def keep_helper_sources(dep_info, out, plugin, mge, plugin_commit, mge_commit, set_helper=None):
    """The stage HELPER_SOURCES_STAGE: writes the helper's source files of each repo, read from the dep-info file of
    the set's build in the maker's tree, to `out`, with the two commits of the build. Exit 0, or 1 with a FAIL line when
    the file is missing or names no source of a repo: the plans would then not know the helper's sources.
    `set_helper` is the set's numidium.exe (addendum 10 C5): the dep-info is read only when the tree's numidium.exe
    beside it has the same md5. Else the set's part was copied from another set, or the set was there already: the list
    stays, with a line, as a part with the same key was built from the same sources."""
    if set_helper is not None:
        built = os.path.splitext(dep_info)[0] + ".exe"
        if not os.path.isfile(built) or not os.path.isfile(set_helper) or file_md5(built) != file_md5(set_helper):
            say("helper sources: the set's numidium.exe %s is not the build of the maker's tree (%s): the list in %s stays"
                % (set_helper, built, out))
            return 0
    try:
        with open(dep_info, encoding="utf-8") as file:
            text = file.read()
    except OSError as error:
        say("FAIL: no dep-info file of the helper: %s" % error)
        return 1
    sources = sources_by_repo(dep_info_paths(text), {"plugin": plugin, "mge": mge})
    empty = [kind for kind, names in sources.items() if not names]
    if empty:
        say("FAIL: %s names no source of %s" % (dep_info, " or ".join(plugin if kind == "plugin" else mge for kind in empty)))
        return 1
    lines = ["%s\tplugin=%s\tmge=%s" % (HELPER_COMMITS, plugin_commit, mge_commit)]
    lines += ["%s\t%s" % (kind, name) for kind in ("plugin", "mge") for name in sources[kind]]
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out + ".new", "w", encoding="ascii", errors="backslashreplace", newline="\n") as file:
        file.write("".join(line + "\n" for line in lines))
    os.replace(out + ".new", out)
    say("helper sources: %d of nirender-plugin and %d of render-link, of plugin %s and mge %s, in %s"
        % (len(sources["plugin"]), len(sources["mge"]), plugin_commit[:10], mge_commit[:10], out))
    return 0


def read_helper_sources(path):
    """(commits {kind: sha}, {kind: tuple of paths}) of a list that keep_helper_sources wrote, or None when the file is
    missing or not in its form."""
    try:
        with open(path, encoding="ascii", errors="replace") as file:
            rows = [line.rstrip("\r\n").split("\t") for line in file]
    except OSError:
        return None
    if not rows or rows[0][0] != HELPER_COMMITS or len(rows[0]) != 3:
        return None
    commits = dict(part.split("=", 1) for part in rows[0][1:] if "=" in part)
    if set(commits) != {"plugin", "mge"} or any(len(row) != 2 or row[0] not in commits for row in rows[1:]):
        return None
    return commits, {kind: tuple(name for row_kind, name in rows[1:] if row_kind == kind) for kind in commits}


def helper_gate_paths(kind, repo, base, path=None):
    """The helper's source files of the repo of `kind` that plan the gate, and one line for the plan. In place of the
    files, None when the list is not known to hold every source at `base`: no list, a list not in the form, or a list
    of a commit that is not an ancestor of `base`. The round then plans the gate, and its build writes the list."""
    path = HELPER_SOURCES_FILE if path is None else path
    if path is None:
        return (), "helper sources: not read (the self-test)"
    kept = read_helper_sources(path)
    if kept is None:
        return None, "helper sources: no list in %s: the round plans the gate, and its build writes the list" % path
    commits, sources = kept
    listed = commits[kind]
    if resolve(repo, listed) is None or not is_ancestor(repo, listed, base):
        return None, ("helper sources: the list in %s is of the commit %s, which is not an ancestor of the base %s: the "
                      "round plans the gate" % (path, listed[:10], base[:10]))
    return sources[kind], ("helper sources: the list of plugin %s and mge %s; %d files of this repo plan the gate"
                           % (commits["plugin"][:10], commits["mge"][:10], len(sources[kind])))


def round_mwse(name, repo=None, installed=installed_commit):
    """(the MWSE commit of the round's sets, None) or (None, why not): --mwse resolved in the MWSE repository, else the
    commit that every test install's record names (mwse_tree.installed_commit)."""
    if name:
        repo = repo or build_set.SOURCE_REPOS["mwse"]
        found = resolve(repo, name)
        return (found, None) if found else (None, "--mwse %s is no commit of %s" % (name, repo))
    return installed()


# W567 part C: the line of the round's base set when the base pair has no set of its own (equal_parts_base): the set
# that serves, or why the base set is built. The plan prints it, and the log and the close line carry it.
BASE_SET_NOTE = []


def equal_parts_base(plugin_repo, tips, mwse_commit, sets_root=None, drift=None):
    """The base set of a round whose base pair has no set (W567 part C): a kept set with the same render-link and MWSE
    commits whose nirender commit is an ancestor of the base's and whose diff to it touches no part input
    (build_set.NIRENDER_PART_INPUTS), the nearest one by commits. Its parts are the base's but for their build ids,
    which the A/B does not read (instance_ab.py compares pictures and hash lines). Before it, the list of inputs is
    checked against the maker's dep-info files (`drift`, build_set.maker_input_drift): a path of a build outside the
    list, or no file to check, builds the set. (set commits, line), (None, line) when the set is built, or (None, None)
    when the base pair has its own set or is not known."""
    sets_root = sets_root or sets.SETS
    wanted = round_set(tips, mwse_commit)
    if wanted is None or os.path.isdir(set_folder_of(wanted, sets_root)):
        return None, None
    building = "base set: the pair %s has no set, and the round builds it: %%s" % sets.set_name(wanted)
    outside, why = (drift or build_set.maker_input_drift)()
    if outside is None:
        return None, building % ("the part inputs cannot be checked (%s)" % why)
    if outside:
        return None, building % ("the builds read %s, which build_set.NIRENDER_PART_INPUTS does not list" % ", ".join(outside[:5]))
    best = None
    for name in sets.sets_in(sets_root):
        try:
            manifest = sets.read_manifest(os.path.join(sets_root, name))
        except sets.SetError:
            continue
        commits = {repo: (manifest.get("commits", {}).get(repo) or {}).get("commit") for repo in sets.REPOS}
        if manifest.get("kind") != sets.KIND_USUAL or manifest.get("patches") or any(commits[repo] != wanted[repo] for repo in ("render-link", "mwse")):
            continue
        older = commits["nirender"]
        if not older or older == wanted["nirender"] or not is_ancestor(plugin_repo, older, wanted["nirender"]):
            continue
        paths = changed_paths(plugin_repo, older, wanted["nirender"])
        if any(build_set.is_part_input(path) for path in paths):
            continue
        distance = int(git(plugin_repo, "rev-list", "--count", "%s..%s" % (older, wanted["nirender"])))
        if best is None or distance < best[0]:
            best = (distance, commits, name, len(paths))
    if best is None:
        return None, building % "no kept set has equal parts"
    distance, commits, name, count = best
    return commits, "base set: %s, with parts equal to the pair %s: nirender %s to %s is %d paths in %d commits, none a part input" % (
        name, sets.set_name(wanted), commits["nirender"][:10], wanted["nirender"][:10], count, distance)


def base_set_lines(stages):
    """BASE_SET_NOTE when the round has the base set stage, for the log and the close line."""
    return list(BASE_SET_NOTE) if any(name == BASE_STAGE for name, _, _ in stages) else []


def planned_stages(kind, repo, paths, baseline, commits, base, sibling_base=None, mwse_commit=None, label=None, mwse_why=None, reuse_base=False):
    """The stages of a check of `paths` from `base`, as main() runs them: with the replay gate, also the base set of
    tips_for and the A/B. (stages, tips, None); (stages, None, why) when the round makes a set whose commits are not known
    (`mwse_why` says why the MWSE commit is not), or when the gate needs a base pair that tips_for cannot give. The
    helper's sources (helper_gate_paths) plan the gate too."""
    helper_paths, _ = helper_gate_paths(kind, repo, base)

    def plan(tips=None, base_commits=None):
        return stages_for(kind, repo, paths, baseline, tips, mwse_commit, commits=commits, base=base, helper_paths=helper_paths, label=label,
                          base_commits=base_commits)

    stages = plan()
    if any(name == SET_STAGE for name, _, _ in stages) and round_set(commits, mwse_commit) is None:
        unknown = "the sibling's commit is not known" if not commits else "its MWSE commit is not known (%s): give --mwse <commit>" % (mwse_why or "none")
        return stages, None, "the round makes a set, and %s" % unknown
    if not any(name == "replay gate" for name, _, _ in stages):
        return stages, None, None
    tips, why = tips_for(kind, repo, base, sibling_base)
    if why:
        return stages, None, why
    if not reuse_base:
        return plan(tips), tips, None
    # The round's own plan (main) may take the set of an equal-parts ancestor as its base set (W567 part C); the plan of
    # the sibling's stages (move_sibling_ref) keeps the pair's own set.
    plugin = repo if kind == "plugin" else sibling_of(kind, repo)
    base_commits, note = equal_parts_base(plugin, tips, mwse_commit)
    del BASE_SET_NOTE[:]
    if note:
        BASE_SET_NOTE.append(note)
    return plan(tips, base_commits), tips, None


def stage_key(stage):
    """What a stage runs: its folder and its command. A name is no proof, as both repos have a stage "checks". The label
    after --in-hold names the round's take, not what the stage runs: it is left out, so that the sibling's own check and
    this round match on the same set (W370)."""
    _, folder, command = stage
    command = list(command)
    if IN_HOLD in command:
        del command[command.index(IN_HOLD):command.index(IN_HOLD) + 2]
    return os.path.normcase(os.path.abspath(folder)), tuple(command)


def missing_stages(needed, ran, results):
    """The names of the stages of `needed` that no stage of `ran` matches by stage_key with a pass in `results`."""
    passed = {stage_key(stage) for stage, result in zip(ran, results) if result}
    return [name for name, folder, command in needed if stage_key((name, folder, command)) not in passed]


def move_sibling_ref(kind, sibling, sibling_head, stages, results, baseline, commits, sibling_changed=False, log_path=None, mwse_commit=None):
    """After a pass of the repo of `kind`: moves the sibling's refs/merge-step/<branch> to `sibling_head`, the
    sibling's HEAD at the start of the round, only when the round ran and passed every stage that the sibling's own
    check of its diff since that ref would run (W370). Else the ref stays, and one line says why. `sibling_changed`:
    the sibling had tracked changes at the start of the round. The move names the old value, so that a ref that
    another check moved meanwhile stays as it is, with a line. The line also goes to `log_path`. True when the ref
    moved."""
    other = "mge" if kind == "plugin" else "plugin"
    name = os.path.basename(sibling)
    branch = branch_of(sibling)
    if not branch:
        ref_say(log_path, "the sibling %s: its merge-step ref stays: the tree is on no branch" % name)
        return False
    ref = "refs/merge-step/%s" % branch
    last = resolve(sibling, ref)
    head = resolve(sibling, "HEAD")
    if last is None:
        ref_say(log_path, "the sibling %s: %s stays absent: without it the diff of the sibling is not known" % (name, ref))
        return False
    if sibling_head is None or head != sibling_head:
        ref_say(log_path, "the sibling %s: %s stays at %s: the HEAD of the sibling moved during the round" % (name, ref, last[:10]))
        return False
    # A plain run, as for the update below: a failing status keeps the ref with a line.
    status = subprocess.run(["git", "-C", sibling, "status", "--porcelain", "--untracked-files=no"], capture_output=True, text=True, env=git_env())
    if status.returncode != 0:
        ref_say(log_path, "the sibling %s: %s stays at %s: git status of the sibling failed (%s)" % (name, ref, last[:10], status.stderr.strip()))
        return False
    changed_now = bool(status.stdout.strip())
    if sibling_changed or changed_now:
        when = "at the start of the round" if sibling_changed else "now"
        ref_say(log_path, "the sibling %s: %s stays at %s: the sibling has tracked changes %s, and the docs and contract stages read its working "
            "tree, not its HEAD" % (name, ref, last[:10], when))
        return False
    if last == head:
        ref_say(log_path, "the sibling %s: %s is at its HEAD %s: nothing to move" % (name, ref, head[:10]))
        return False
    if not is_ancestor(sibling, last, head):
        ref_say(log_path, "the sibling %s: %s stays at %s: it is not an ancestor of the HEAD %s" % (name, ref, last[:10], head[:10]))
        return False
    paths = changed_paths(sibling, last, head)
    # The same MWSE commit as this round: the sibling's own check would make the same set.
    needed, _, why = planned_stages(other, sibling, paths, baseline, commits, last, mwse_commit=mwse_commit)
    if why:
        ref_say(log_path, "the sibling %s: %s stays at %s: the stages of its diff to %s are not known (%s)" % (name, ref, last[:10], head[:10], why))
        return False
    missing = missing_stages(needed, stages, results)
    if missing:
        ref_say(log_path, "the sibling %s: %s stays at %s: its diff to %s needs %s, which did not run and pass in this round; check it on its own"
            % (name, ref, last[:10], head[:10], ", ".join(missing)))
        return False
    # A plain run, not git(): its SystemExit would end a round that passed with exit code 1.
    update = subprocess.run(["git", "-C", sibling, "update-ref", ref, head, last], capture_output=True, text=True, env=git_env())
    if update.returncode != 0:
        now = resolve(sibling, ref)
        ref_say(log_path, "the sibling %s: %s stays at %s: git update-ref refused the move from %s to %s (%s)"
            % (name, ref, now[:10] if now else "nothing", last[:10], head[:10], update.stderr.strip()))
        return False
    ref_say(log_path, "the sibling %s: %s moves from %s to %s: this round ran and passed every stage of that diff" % (name, ref, last[:10], head[:10]))
    return True


def scratch_slots(state):
    """slot.py on a scratch state folder, as a module of its own: it reads SLOT_DIR and the process names when it loads. A
    fake session holds the slots, and the names of the game and the compilers match no process, so that a real game or a
    real compile cannot refuse a take of the test. The environment is the test's to keep."""
    os.makedirs(state, exist_ok=True)
    os.environ.update(SLOT_DIR=state, SLOT_GAME_IMAGE="no-such-program.exe", SLOT_COMPILERS="no-such-program.exe", SLOT_PROJECT_FOLDERS="",
                      SLOT_FREE_MEMORY_GB="64", SLOT_REPLAY_IMAGE="no-such-program.exe", CLAUDE_CODE_SESSION_ID="merge-step-self-test")
    os.environ.pop("SLOT_OVERRIDE", None)
    sys.path.insert(0, HARNESS)
    spec = importlib.util.spec_from_file_location("slot_of_the_self_test", os.path.join(HARNESS, "slot.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # The names match no process, so the test's process list is empty. The real list can fail: tasklist has 5 s, a busy
    # machine can pass that, and with no list an idle hold does not lapse (case 6 failed so once).
    module.running_images = lambda: []
    return module


def self_test_equal_parts(root):
    """The tests of equal_parts_base (W567 part C) on a scratch repo and scratch sets: no E:, no build."""
    import json

    repo = os.path.join(root, "parts", PLUGIN_NAME)
    os.makedirs(repo)
    author = ["-c", "user.name=t", "-c", "user.email=t@example.com"]
    subprocess.run(["git", "-C", repo, "init", "-q", "-b", "master"], check=True, env=git_env())
    commits = []
    for path in ("src/a.rs", "src/b.rs", "tools/x.py", "README.md", "host/src/c.rs"):
        os.makedirs(os.path.dirname(os.path.join(repo, path)) or repo, exist_ok=True)
        with open(os.path.join(repo, path), "w", encoding="ascii") as file:
            file.write(path + "\n")
        subprocess.run(["git", "-C", repo, "add", path], check=True, env=git_env())
        subprocess.run(["git", "-C", repo, *author, "commit", "-q", "-m", path], check=True, env=git_env())
        commits.append(resolve(repo, "HEAD"))
    first, second, tools_change, readme_change, host_change = commits
    link, mwse = "a" * 40, "b" * 40
    sets_root = os.path.join(root, "parts", "sets")

    def kept(nirender, render_link=link, patches=None, kind=sets.KIND_USUAL):
        found = {"nirender": nirender, "render-link": render_link, "mwse": mwse}
        folder = os.path.join(sets_root, sets.set_name(found) if kind == sets.KIND_USUAL else sets.set_name(found, kind=kind))
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, sets.MANIFEST), "w", encoding="utf-8") as file:
            json.dump({"form": sets.FORM, "kind": kind, "patches": patches or {}, "commits": {repo_name: {"commit": commit} for repo_name, commit in found.items()}}, file)
        return folder

    clean = lambda: ([], None)  # noqa: E731
    tips = lambda nirender: {"plugin": nirender, "mge": link}  # noqa: E731
    kept(first)
    kept(second)
    # A tools and README change after `second`: its set serves, the nearest one, with the count of paths and commits.
    found, line = equal_parts_base(repo, tips(readme_change), mwse, sets_root, clean)
    assert found == {"nirender": second, "render-link": link, "mwse": mwse}, (found, line)
    assert line.startswith("base set: %s, with parts equal to the pair " % sets.set_name(found)) and "is 2 paths in 2 commits, none a part input" in line, line
    # The null: a part input (host/src) in the diff, so no kept set serves, and the round builds.
    found, line = equal_parts_base(repo, tips(host_change), mwse, sets_root, clean)
    assert found is None and "has no set, and the round builds it: no kept set has equal parts" in line, line
    # Of two sets with equal parts, the nearer one by commits serves.
    far_root, sets_root = sets_root, os.path.join(root, "parts", "sets-near")
    kept(second)
    kept(tools_change)
    found, line = equal_parts_base(repo, tips(readme_change), mwse, sets_root, clean)
    assert found["nirender"] == tools_change and "is 1 paths in 1 commits" in line, line
    sets_root = far_root
    # The pair's own set: nothing to say.
    kept(readme_change)
    assert equal_parts_base(repo, tips(readme_change), mwse, sets_root, clean) == (None, None)
    # Another render-link commit, a patched set and another kind never serve.
    for other in (dict(render_link="c" * 40), dict(patches={"nirender": "d" * 32})):
        shutil_root = sets_root
        sets_root = os.path.join(root, "parts", "sets-%d" % len(other))
        kept(second, **other)
        assert equal_parts_base(repo, tips(tools_change), mwse, sets_root, clean)[0] is None, other
        sets_root = shutil_root
    # The list of inputs against the truth: a build path outside it, or no dep-info file, builds the set.
    found, line = equal_parts_base(repo, tips(tools_change), mwse, sets_root, lambda: (["tools/gen.rs"], None))
    assert found is None and "the builds read tools/gen.rs, which build_set.NIRENDER_PART_INPUTS does not list" in line, line
    found, line = equal_parts_base(repo, tips(tools_change), mwse, sets_root, lambda: (None, "no dep-info file x"))
    assert found is None and "the part inputs cannot be checked (no dep-info file x)" in line, line
    assert equal_parts_base(repo, tips(tools_change), mwse, sets_root, clean)[0]["nirender"] == second
    # The plan takes it as the base set of the stage and of the A/B, and the line goes to the log and the close.
    plugin = os.path.join(root, "parts", "plugin")
    planned = stages_for("plugin", plugin, ["host/src/material.rs"], BASELINE, {"plugin": "1" * 40, "mge": "2" * 40}, mwse,
                         base_commits={"nirender": second, "render-link": link, "mwse": mwse})
    commands = {name: command for name, _, command in planned}
    assert commands[BASE_STAGE][commands[BASE_STAGE].index("--nirender") + 1] == second, commands[BASE_STAGE]
    ab = commands[INSTANCE_STAGE]
    assert ab[ab.index("--base-build") + 1] == set_folder_of({"nirender": second, "render-link": link, "mwse": mwse}), ab
    try:
        BASE_SET_NOTE[:] = ["base set: x"]
        assert base_set_lines(planned) == ["base set: x"] and base_set_lines(stages_for("plugin", plugin, ["tools/x.py"])) == []
    finally:
        del BASE_SET_NOTE[:]


def self_test_gate_list(root, plugin):
    """The tests of --gate-list (W567 part D): the option, the gate's argument, the accept as the round's last stage, the
    close line, and the stop that keeps the accept from a round that failed."""
    listed = os.path.join(root, "gate-list.txt")
    with open(listed, "w", encoding="utf-8") as file:
        file.write("tour/3-03-_nif_052.png\n")
    assert gate_list_of(None, BASELINE) == []
    assert "is no file" in gate_list_of(os.path.join(root, "none.txt"), BASELINE)
    assert "goes with no --baseline" in gate_list_of(listed, os.path.join(root, "copy"))
    assert gate_list_of(listed, BASELINE) == [(os.path.abspath(listed), file_md5(listed))]
    tips = {"plugin": "1" * 40, "mge": "2" * 40}
    try:
        GATE_LIST[:] = gate_list_of(listed, BASELINE)
        planned = stages_for("plugin", plugin, ["host/src/material.rs"], BASELINE, tips)
        commands = {name: command for name, _, command in planned}
        gate = commands["replay gate"]
        assert gate[-2:] == ["--gate-list", os.path.abspath(listed)], gate
        assert planned[-1][0] == ACCEPT_LIST_STAGE, [name for name, _, _ in planned]
        accept = commands[ACCEPT_LIST_STAGE]
        assert accept[3:6] == ["accept-list", "--gate-list", os.path.abspath(listed)], accept
        assert accept[accept.index("--now") + 1] == gate[gate.index("--now") + 1] and accept[accept.index("--mge-build") + 1] == gate[gate.index("--mge-build") + 1], (accept, gate)
        assert "gate list %s md5 %s accepted" % (os.path.abspath(listed), file_md5(listed)) in gate_list_close(planned)
        # No gate: no accept, and the close names no list.
        plain = stages_for("plugin", plugin, ["tools/x.py"])
        assert ACCEPT_LIST_STAGE not in [name for name, _, _ in plain] and gate_list_close(plain) == []
        # The accept runs only after every stage passed: a red gate stops the round before it.
        good, bad = [sys.executable, "-c", "pass"], [sys.executable, "-c", "raise SystemExit(1)"]
        never = [sys.executable, "-c", "raise SystemExit(9)"]
        now = os.path.join(root, "gate-list-now")
        os.makedirs(os.path.join(now, "tour"), exist_ok=True)
        with open(os.path.join(root, "log-gate-list.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("replay gate", root, bad + ["--now", now]), (ACCEPT_LIST_STAGE, root, never)], log) == [False, False]
            assert STAGE_WORDS[-1] == (ACCEPT_LIST_STAGE, NOT_RUN_WORD, "replay gate did not pass, and the round stops at its first failure"), STAGE_WORDS
            assert run_stages([("replay gate", root, good + ["--now", now]), (ACCEPT_LIST_STAGE, root, good)], log) == [True, True]
    finally:
        del GATE_LIST[:]
    assert ACCEPT_LIST_STAGE not in [name for name, _, _ in stages_for("plugin", plugin, ["host/src/material.rs"], BASELINE, tips)], "a round with no list accepts"


def self_test_quick(root, plugin, mge, base):
    """The tests of --quick (W567 part A): its stages, its refusals and its line. `plugin` and `mge` are the self-test's
    trees, and `base` the names of the plugin's fast stages."""
    stages, why = quick_plan("plugin", plugin, mge, "abc")
    assert why is None and [name for name, _, _ in stages] == base, stages
    commands = {name: command for name, _, command in stages}
    assert commands["docs"][-2:] == ["--mge", mge] and commands["checks"][-2:] == ["--base", "abc"], commands
    assert stages == fast_stages("plugin", plugin, mge, "abc") and stages == stages_for("plugin", plugin, ["tools/x.py"], base="abc"), "a quick round runs other fast stages than a merge check"
    # A plugin tree with no render-link tree beside it checks its docs without the helper (docs.py, W510).
    alone, why = quick_plan("plugin", plugin, os.path.join(root, "no-render-link"), None)
    assert why is None and "--mge" not in dict((name, command) for name, _, command in alone)["docs"] and "--base" not in dict((name, command) for name, _, command in alone)["checks"]
    # A render-link tree runs its stages with the tools of a plugin tree, and needs one.
    stages, why = quick_plan("mge", mge, plugin, "abc")
    assert why is None and stages == fast_stages("mge", plugin, mge, "abc") and dict((name, folder) for name, folder, _ in stages)["clippy baseline"] == plugin, stages
    stages, why = quick_plan("mge", mge, os.path.join(root, "none"), None)
    assert stages is None and "--sibling <a %s tree>" % PLUGIN_NAME in why, why
    assert quick_plan(None, root, "", None)[0] is None
    # The line: a pass names every stage; a failure names the first stage that did not pass, with its word and reason.
    head = quick_head("r", "c" * 40)
    assert head == "r cccccccccc (no sibling)" and quick_head("r", "c" * 40, ("s", "e" * 40)) == "r cccccccccc (sibling s eeeeeeeeee)"
    words = [("a", PASS_WORD, ""), ("b", PASS_WORD, "")]
    assert quick_line(head, words, "L") == "QUICK PASS r cccccccccc (no sibling): 2 of 2 (a, b); log L", quick_line(head, words, "L")
    words = [("a", PASS_WORD, ""), ("b", FAIL_WORD, "exit code 101"), ("c", NOT_RUN_WORD, "b did not pass")]
    assert quick_line(head, words, "L") == "QUICK FAIL r cccccccccc (no sibling): b (FAIL: exit code 101); log L"
    assert quick_line(head, [], "L") == "QUICK FAIL r cccccccccc (no sibling): no stage ran; log L"
    # The label names the tool (slot.py's rule of D504) and does not go ahead of the merge checks (D148).
    label = QUICK_LABEL % (PLUGIN_NAME, "c" * 10)
    assert not label.startswith("merge_step:") and label.split(":", 1)[0].strip().endswith(".py"), label
    # The refusals of a real tree, and the plan of --dry-run, which takes no slot.
    tree = os.path.join(root, "quick", PLUGIN_NAME)
    os.makedirs(os.path.join(tree, "host", "src"))
    author = ["-c", "user.name=t", "-c", "user.email=t@example.com"]
    with open(os.path.join(tree, "host", "src", "shader.wgsl"), "w", encoding="ascii") as file:
        file.write("a\n")
    for command in (["init", "-q", "-b", "side"], ["add", "host/src/shader.wgsl"], author + ["commit", "-q", "-m", "one"]):
        subprocess.run(["git", "-C", tree, *command], check=True, env=git_env())
    args = types.SimpleNamespace(repo=tree, base=None, sibling=None, dry_run=True, no_wait=True)
    said = io.StringIO()
    with contextlib.redirect_stdout(said):
        assert run_quick(args) == 1
    assert "QUICK FAIL %s " % PLUGIN_NAME in said.getvalue() and "no branch master" in said.getvalue(), said.getvalue()
    subprocess.run(["git", "-C", tree, "branch", "-q", "master"], check=True, env=git_env())
    head = resolve(tree, "HEAD")
    said = io.StringIO()
    with contextlib.redirect_stdout(said):
        assert run_quick(args) == 0
    assert said.getvalue().startswith("quick: %s %s (no sibling), base %s, sibling none (the docs skip the helper); stages: %s" % (PLUGIN_NAME, head[:10], head[:10], ", ".join(base))), said.getvalue()
    # A render-link tree beside it: the docs build the helper there, so the line names its commit, and a sibling with
    # tracked changes is refused (verdict 605).
    beside = os.path.join(root, "quick", MGE_NAME)
    os.makedirs(os.path.join(beside, "mgeHost64"))
    with open(os.path.join(beside, "mgeHost64", "lib.rs"), "w", encoding="ascii") as file:
        file.write("a\n")
    for command in (["init", "-q"], ["add", "mgeHost64/lib.rs"], author + ["commit", "-q", "-m", "one"]):
        subprocess.run(["git", "-C", beside, *command], check=True, env=git_env())
    said = io.StringIO()
    with contextlib.redirect_stdout(said):
        assert run_quick(args) == 0
    assert said.getvalue().startswith("quick: %s %s (sibling %s %s), base %s, sibling %s; stages: " % (
        PLUGIN_NAME, head[:10], MGE_NAME, resolve(beside, "HEAD")[:10], head[:10], beside)), said.getvalue()
    with open(os.path.join(beside, "mgeHost64", "lib.rs"), "w", encoding="ascii") as file:
        file.write("b\n")
    said = io.StringIO()
    with contextlib.redirect_stdout(said):
        assert run_quick(args) == 1
    assert "QUICK FAIL %s %s (sibling %s " % (PLUGIN_NAME, head[:10], MGE_NAME) in said.getvalue() and "has tracked changes" in said.getvalue(), said.getvalue()
    # The null: --sibling to a folder with no render-link tree passes the same plugin tree again.
    said = io.StringIO()
    with contextlib.redirect_stdout(said):
        assert run_quick(types.SimpleNamespace(repo=tree, base=None, sibling=os.path.join(root, "none"), dry_run=True, no_wait=True)) == 0
    assert "(no sibling)" in said.getvalue(), said.getvalue()
    with open(os.path.join(tree, "host", "src", "shader.wgsl"), "w", encoding="ascii") as file:
        file.write("b\n")
    said = io.StringIO()
    with contextlib.redirect_stdout(said):
        assert run_quick(args) == 1
    assert "the tree has tracked changes" in said.getvalue(), said.getvalue()
    said = io.StringIO()
    with contextlib.redirect_stdout(said):
        assert run_quick(types.SimpleNamespace(repo=root, base=None, sibling=None, dry_run=True, no_wait=True)) == 1
    assert "is no tree of" in said.getvalue(), said.getvalue()
    # The log of a quick round is in the tree, not on E: (D297).
    assert log_path_of("d" * 40, os.path.join(tree, "target"), "quick") == os.path.join(tree, "target", "quick-dddddddddd.log")
    assert log_path_of("d" * 40, os.path.join(tree, "target"), "quick") == os.path.join(tree, "target", "quick-dddddddddd-2.log")


def self_test():
    global HELPER_SOURCES_FILE
    HELPER_SOURCES_FILE = None
    with tempfile.TemporaryDirectory() as root:
        plugin = os.path.join(root, PLUGIN_NAME)
        mge = os.path.join(root, MGE_NAME)
        os.makedirs(os.path.join(plugin, "host", "src"))
        with open(os.path.join(plugin, "host", "src", "shader.wgsl"), "w", encoding="ascii"):
            pass
        os.makedirs(os.path.join(mge, "mgeHost64"))
        assert repo_kind(plugin) == "plugin" and repo_kind(mge) == "mge" and repo_kind(root) is None

        def names(kind, repo, *paths):
            return [name for name, _, _ in stages_for(kind, repo, paths)]

        base = ["plugin tests", "host tests", HOST_GPU_STAGE, "clippy baseline", "checks", "docs"]
        # W521: every round that deploys makes its set first (the maker probes the set's MWSE.dll before the seal, G1), then
        # deploys it. A round of the plugin DLL or the mod's Lua deploys without the gate.
        gate_tail = [SET_STAGE, DEPLOY_STAGE, "replay gate", HELPER_SOURCES_STAGE]
        deploy_tail = [SET_STAGE, DEPLOY_STAGE]
        assert names("plugin", plugin, "tools/x.py") == base and names("plugin", plugin, "README.md") == base, "a change of no deployed part deployed"
        assert names("plugin", plugin, "src/lib.rs") == base + deploy_tail
        assert names("plugin", plugin, "build.rs") == names("plugin", plugin, "build_support/build_id.rs") == base + deploy_tail
        assert names("plugin", plugin, "tools/x.py", "src/lib.rs") == base + deploy_tail
        # A change of the host crate's source also runs the helper's tests in the sibling (W523, the audit's F-H-2).
        host_base = base + [HELPER_TEXT_STAGE]
        assert names("plugin", plugin, "host/src/shader.wgsl") == host_base + gate_tail
        assert names("plugin", plugin, "host/src/material.rs") == host_base + gate_tail
        # W535: a change of a file of the table's order adds the order stage right after the gate; no other change does.
        order_tail = [SET_STAGE, DEPLOY_STAGE, "replay gate", ORDER_STAGE, HELPER_SOURCES_STAGE]
        for path in ORDER_PATHS["plugin"]:
            assert names("plugin", plugin, path) == host_base + order_tail, path
        assert names("plugin", plugin, "host/src/retained.rs", "src/lib.rs") == host_base + order_tail, "the gate round deploys once"
        assert names("plugin", plugin, "host/src/retained_old.rs") == host_base + gate_tail, "a name that only starts the same ran the order stage"
        assert ORDER_STAGE not in names("mge", mge, "mgeHost64/src/renderer.rs") and ORDER_STAGE in GAME_STAGES
        order_command = {name: command for name, _, command in stages_for("plugin", plugin, ["host/src/switches.rs"])}[ORDER_STAGE]
        assert order_command[3:5] == ["order", "--mge-build"] and order_command[-1] == os.path.join(plugin, "target", "merge-step-order"), order_command
        self_test_order_drift()
        assert names("plugin", plugin, "host/src_old/x.rs") == base, "a name that only starts the same ran the gate"
        assert names("plugin", plugin, "srcx/lib.rs") == base, "a name that only starts the same deployed"
        assert names("plugin", plugin, "contract/mge_render_link.h") == base + ["d3d8 build", "contract"] + deploy_tail
        self_test_quick(root, plugin, mge, base)
        self_test_gate_list(root, plugin)
        self_test_equal_parts(root)
        # The set of the commits under test and the round's MWSE commit: the maker in the round's take (--in-hold, C2), in
        # the plugin's tree; the deploy and the gate name its folder (C3).
        full = under_test("plugin", "a" * 40, "b" * 40)
        mwse = "c" * 40
        the_set = os.path.join(sets.SETS, "s-aaaaaaaa-bbbbbbbb-cccccccc")
        assert set_folder_of(round_set(full, mwse)) == the_set and round_set(full, None) is None and set_folder_of(None) == UNPLANNED_SET
        planned = {name: (folder, command) for name, folder, command in stages_for("plugin", plugin, ["host/src/material.rs"], commits=full, mwse_commit=mwse, label="merge_step: x")}
        maker = planned[SET_STAGE]
        assert maker[0] == plugin and maker[1][2:] == [os.path.join("tools", "build_set.py"), "--nirender", "a" * 40, "--render-link", "b" * 40, "--mwse", mwse,
                                                       IN_HOLD, "merge_step: x"], maker
        deploy_command = planned[DEPLOY_STAGE][1]
        assert deploy_command[deploy_command.index("--set") + 1] == the_set and "--build" not in deploy_command, deploy_command
        gate_command = planned["replay gate"][1]
        assert gate_command[gate_command.index("--mge-build") + 1] == the_set, gate_command
        assert not any("dist-staging" in part or "base-" in os.path.basename(part) for _, command in planned.values() for part in command), planned
        # C5: the helper sources read the maker's render-link tree and compare the set's numidium.exe.
        sources = planned[HELPER_SOURCES_STAGE][1]
        assert sources[4] == os.path.join(build_set.TREES, build_set.TREE_NAMES["render-link"], HELPER_DEP_INFO) and sources[-1] == os.path.join(the_set, "numidium.exe"), sources
        # The label of the take is no part of what a stage runs (W370 compares the sibling's stages by stage_key).
        assert stage_key((SET_STAGE, plugin, maker[1])) == stage_key((SET_STAGE, plugin, set_stage(SET_STAGE, plugin, round_set(full, mwse), "other")[2]))
        assert stage_key((SET_STAGE, plugin, maker[1])) != stage_key((SET_STAGE, plugin, set_stage(SET_STAGE, plugin, round_set(full, "d" * 40), "merge_step: x")[2]))
        # A change of the ports, the hooks, the install or the table: the MWSE.dll of the round's set, the one that the deploy
        # writes, after the set and before the deploy (addendum 11). A change of the baseline alone deploys nothing: the
        # MWSE.dll of each test install, as before.
        dlls = [("Morrowind75testing", "D:/m75/MWSE.dll"), ("Outlander-test", "D:/o/MWSE.dll")]
        sites = ["mwse sites Morrowind75testing", "mwse sites Outlander-test"]
        for path in ("src/ports/device_setup.rs", "src/hook.rs", "src/install.rs", "src/gen/funcs.rs", "src/site.rs"):
            stages = stages_for("plugin", plugin, [path], mwse_dlls=dlls, commits=full, mwse_commit=mwse)
            assert [name for name, _, _ in stages] == base + [SET_STAGE, SITES_SET_STAGE, DEPLOY_STAGE], path
            set_site = dict((name, command) for name, _, command in stages)[SITES_SET_STAGE]
            assert set_site[-2:] == [os.path.join("tools", "check_mwse_sites.py"), os.path.join(the_set, "MWSE.dll")], set_site
            assert not any(dll in command for _, _, command in stages for _, dll in dlls), "an install's MWSE.dll was read in a round with a set"
        stages = stages_for("plugin", plugin, ["tools/mwse_sites.baseline"], mwse_dlls=dlls)
        assert [name for name, _, _ in stages] == base + sites and stages[len(base) + 1][2][-2:] == [os.path.join("tools", "check_mwse_sites.py"), "D:/o/MWSE.dll"]
        assert stages[len(base) + 1][1] == plugin and SET_STAGE not in [name for name, _, _ in stages], "the null: no set, each install's MWSE.dll"
        assert [name for name, _, _ in stages_for("plugin", plugin, ["src/lib.rs"], mwse_dlls=dlls)] == base + deploy_tail
        assert [name for name, _, _ in stages_for("mge", mge, ["src/hook.rs"], mwse_dlls=dlls)] == ["helper tests", GPU_STAGE, "clippy baseline", "checks", "docs"]
        assert not any(name.startswith("mwse sites") for kind, repo, path in (("mge", mge, "mgeHost64/src/renderer.rs"), ("plugin", plugin, "src/lib.rs"),
                                                                              ("plugin", plugin, "host/src/material.rs")) for name in names(kind, repo, path))
        # The deployed DLL, never the copy of a live run in a Harness mod (2026-11-10: the stage read Ryker's Brief 6 DLL,
        # which sorts before mods/MWSE).
        assert not any(HARNESS_MOD.search(path) for path in MWSE_DEPLOYS.values()), MWSE_DEPLOYS
        assert [name for name, _ in deployed_mwse_dlls()] == list(MWSE_DEPLOYS)
        deploys = {"A": "D:/m/A/mods/MWSE/Root/MWSE.dll", "B": "D:/m/B/mods/missing/Root/MWSE.dll"}
        assert deployed_mwse_dlls(deploys) == list(deploys.items()), "a missing deploy left the list: its stage did not run"
        for harness in ("D:/m/A/mods/Harness - nirender/Root/MWSE.dll", "D:\\m\\A\\mods\\harness - other\\Root\\MWSE.dll"):
            try:
                deployed_mwse_dlls({"A": harness})
            except ValueError as error:
                assert "Harness mod" in str(error), error
            else:
                raise AssertionError("a Harness copy was read: " + harness)
        # The mod's Lua: the set holds it (part plugin-lua), and since W406 a change of it is deployed.
        assert names("plugin", plugin, "mod/MWSE/mods/nirender/main.lua") == base + deploy_tail
        assert names("plugin", plugin, "mod/MWSE/mods/nirender/main.lua", "host/src/material.rs") == host_base + gate_tail
        assert needs_clean_sibling(stages_for("plugin", plugin, ["mod/MWSE/mods/nirender/noplugin.lua"]))
        for path in ("Cargo.toml", "Cargo.lock", "host/Cargo.toml", "host/Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "host/.cargo/config.toml"):
            assert names("plugin", plugin, path) == base + gate_tail, path
        # W528 item 5: a change of a hook fixture plans the gate, whose second pass reads the fixtures.
        assert names("plugin", plugin, "tools/hook-fixtures/XE Mod Water.wgsl")[-len(gate_tail):] == gate_tail, names("plugin", plugin, "tools/hook-fixtures/XE Mod Water.wgsl")
        assert not names("plugin", plugin, "tools/merge_step.py")[-len(gate_tail):] == gate_tail, "the null: another tool plans no gate"
        mge_base = ["helper tests", GPU_STAGE, "clippy baseline", "checks", "docs"]
        assert names("mge", mge, "xtask/src/main.rs") == mge_base + ["build of the dist"]
        for path in ("Cargo.toml", "Cargo.lock", "mgeHost64/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"):
            assert names("mge", mge, path) == mge_base + gate_tail, path
        assert names("mge", mge, "mge-config/Cargo.toml") == mge_base + ["d3d8 build", "contract"] + gate_tail
        # A render-link round makes the set of the pair in the plugin's tree too, with the plugin's commit under test.
        mge_full = under_test("mge", "b" * 40, "a" * 40)
        mge_maker = {name: (folder, command) for name, folder, command in stages_for("mge", mge, ["mgeHost64/src/renderer.rs"], commits=mge_full, mwse_commit=mwse)}[SET_STAGE]
        assert mge_maker[0] == plugin and mge_maker[1][3:9] ==["--nirender", "a" * 40, "--render-link", "b" * 40, "--mwse", mwse], mge_maker
        assert stages_for("mge", mge, [])[0][2][-4:] == ["-p", "mgeHost64", "-p", "mge-config"], stages_for("mge", mge, [])[0][2]
        # The GPU tests run the ignored tests of the helper in its tree, through the wrapper that fails on 0 tests, and the
        # plugin's tests do not run them.
        gpu_folder, gpu_command = stages_for("mge", mge, [])[1][1:]
        assert gpu_folder == mge and gpu_command[3:] == [REQUIRE_TESTS, "cargo", "test", "--quiet", "-p", "mgeHost64", "--", "--ignored", "--test-threads=1"]
        assert gpu_command[:2] == [sys.executable, "-I"] and gpu_command[2] == os.path.join(plugin, "tools", "merge_step.py")
        assert all(name != GPU_STAGE for name, _, _ in stages_for("plugin", plugin, []))
        # The host's GPU tests run in the host's folder, with the child-only tests (which dump this process; their names end in _in_this_process) left out, and the
        # helper's stage is not in the plugin's list or the other way round.
        host_folder, host_command = stages_for("plugin", plugin, [])[2][1:]
        assert stages_for("plugin", plugin, [])[2][0] == HOST_GPU_STAGE and host_folder == os.path.join(plugin, "host")
        assert host_command[3:] == [REQUIRE_TESTS, "cargo", "test", "--quiet", "--", "--ignored", "--skip", "_in_this_process", "--test-threads=1"], host_command
        assert all(name != HOST_GPU_STAGE for name, _, _ in stages_for("mge", mge, []))
        assert stages_for("mge", mge, [])[2][2][-2:] == ["--mge", mge]
        assert names("mge", mge, "mge-config/src/lib.rs") == mge_base + ["d3d8 build", "contract"]
        assert names("mge", mge, "mgeHost64/src/renderer/water.rs") == mge_base + gate_tail
        assert names("mge", mge, "mgeHost64/src/renderer.rs") == mge_base + gate_tail
        assert names("mge", mge, "assets/Data Files/shaders/core/x.fx") == mge_base + gate_tail
        assert names("mge", mge, "assets/Data Files/other.txt") == mge_base
        assert names("mge", mge, "mgeHost64/src/other.rs") == mge_base
        assert names("mge", mge, "d3d8/cpp/mge/x.cpp") == mge_base + ["d3d8 build", "contract"]

        # Each stage runs in the right repo: the gate replays the build of the sibling, and the checks of the tools' repo.
        stages = {name: (folder, command) for name, folder, command in stages_for("mge", mge, ["mgeHost64/src/renderer.rs", "d3d8/x"])}
        assert stages["checks"][0] == plugin and stages["checks"][1][-1] == mge
        assert stages["replay gate"][0] == plugin and UNPLANNED_SET in stages["replay gate"][1], "no commits: the plan names no set"
        assert stages["d3d8 build"][0] == mge and stages["contract"][1][-1] == mge

        # The gate compares with the folder that it is given, and neither the accepted folder nor a copy takes its pictures.
        for folder, given in ((BASELINE, ()), (os.path.join(root, "copy"), (os.path.join(root, "copy"),))):
            command = {name: command for name, _, command in stages_for("plugin", plugin, ["host/src/shader.wgsl"], *given)}["replay gate"]
            assert command[command.index("--baseline") + 1] == folder and "--accept" not in command
            assert not command[command.index("--now") + 1].startswith(folder + os.sep), folder

        # The state of the tree.
        plain = stages_for("plugin", plugin, ["tools/x.py"])
        gate = stages_for("plugin", plugin, ["host/src/shader.wgsl"])
        os.makedirs(os.path.join(mge, "target"))
        assert tree_problem("plugin", plugin, plain, "abc", "abc", False) is None
        assert "not" in tree_problem("plugin", plugin, plain, "abc", "def", False)
        assert "tracked changes" in tree_problem("plugin", plugin, plain, "abc", "abc", True)
        assert "no tree" in tree_problem(None, root, [], "abc", "abc", False)
        assert tree_problem("plugin", plugin, gate, "abc", "abc", False) is None
        # W521: the maker builds the gate's and the deploy's set in its own trees of the commits, so a worktree of another
        # name may run them (before, the gate and the dist built this tree beside its sibling).
        worktree = os.path.join(root, PLUGIN_NAME + "-x")
        os.makedirs(os.path.join(worktree, "host"))
        assert tree_problem("plugin", worktree, stages_for("plugin", worktree, ["host/src/shader.wgsl"]), "abc", "abc", False) is None
        assert tree_problem("plugin", worktree, stages_for("plugin", worktree, ["src/lib.rs"]), "abc", "abc", False) is None
        assert tree_problem("plugin", plugin, stages_for("plugin", plugin, ["src/lib.rs"]), "abc", "abc", False) is None
        assert "tracked changes" in tree_problem("plugin", plugin, stages_for("plugin", plugin, ["src/lib.rs"]), "abc", "abc", False, True)
        # Without the gate, a worktree of another name passes; a missing sibling does not.
        assert tree_problem("plugin", worktree, [("a", plugin, [])], "abc", "abc", False) is None
        assert "sibling" in tree_problem("plugin", plugin, [("a", os.path.join(root, "gone"), [])], "abc", "abc", False)
        # A change in the sibling stops the stages that build from it, and only these.
        assert "tracked changes" in tree_problem("plugin", plugin, gate, "abc", "abc", False, True)
        d3d8 = stages_for("plugin", plugin, ["contract/x.h"])
        assert "tracked changes" in tree_problem("plugin", plugin, d3d8, "abc", "abc", False, True)
        assert tree_problem("plugin", plugin, plain, "abc", "abc", False, True) is None

        # The base. An older base than the tip before the move only widens the diff.
        assert base_problem("abc", "def", True, "abc", True) is None
        assert base_problem("old", "def", True, "abc", True) is None
        assert "empty" in base_problem("abc", "abc", True, "abc", True)
        assert "no ancestor" in base_problem("abc", "def", False, "abc", True)
        # A fast-forward of two commits with --base HEAD^: the base is newer than the tip before the merge.
        assert "skip a part" in base_problem("head^", "head", True, "abc", False)
        assert "no <branch>@{1}" in base_problem("abc", "def", True, None, False)
        # A later commit moved the tip past a merge that failed or had no run.
        assert "that failed or had no run" in base_problem("merge", "later", True, "merge", True, "passed", False)
        assert base_problem("passed", "later", True, "merge", True, "passed", True) is None

        # A stage's build stays in its tree.
        moved = stage_env({"PATH": "p", "OTHER": "o", "CARGO_TARGET_DIR": "t", "CARGO_BUILD_TARGET_DIR": "t", "NIRENDER_PLUGIN_DIR": "d"})
        assert moved["OTHER"] == "o" and moved["PATH"].endswith(os.pathsep + "p") and not set(BUILD_MOVERS) & set(moved)
        # The plugin's tests in a round get the switch and their inputs (W527); no other stage does (the null).
        keys = {"mcpatch_root": "D:/m", "vanilla_exe": "D:/v.exe"}
        inputs = tests_inputs(keys.get, {"first": "D:/a/MWSE.dll", "second": "D:/b/MWSE.dll"})
        assert inputs == {TESTS_NEED_INPUTS: "1", "NIRENDER_MCPATCH": "D:/m", "NIRENDER_VANILLA_EXE": "D:/v.exe", TESTS_MWSE_DLL: "D:/a/MWSE.dll"}, inputs
        assert tests_inputs({}.get, {}) == {TESTS_NEED_INPUTS: "1"}, "a missing key stays unset, and its test fails"
        assert stage_env({"PATH": "p"}, TESTS_STAGE, inputs)[TESTS_NEED_INPUTS] == "1"
        assert TESTS_NEED_INPUTS not in stage_env({"PATH": "p"}, "host tests", inputs)
        assert stages_for("plugin", plugin, ["src/lib.rs"])[0][0] == TESTS_STAGE

        # A stage's own exit code decides its line, and the log has its output.
        with open(os.path.join(root, "log.txt"), "w", encoding="utf-8") as log:
            assert run_stage("good", root, [sys.executable, "-c", "print('out')"], log)
            assert not run_stage("bad", root, [sys.executable, "-c", "raise SystemExit(4)"], log)
        with open(os.path.join(root, "log.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "out" in text
        # The verdict of each stage is in the log too, so that a kept log shows it.
        assert "== PASS good (" in text and "== FAIL bad (" in text and ": exit code 4" in text, text
        assert "refused" not in text, "a stage with no refusal names none"
        assert "FALLBACK" not in text, "a stage with no MWSE fallback names none"
        assert summary_line([True, False, True], "x.log", []) == "2 of 3 stages passed; the output is in x.log", "words not known"
        # Verdict 467: a WITHHELD stage is not counted as a pass.
        words = [("a", PASS_WORD, ""), ("b", FAIL_WORD, "x"), ("c", WITHHELD_WORD, "a game runs")]
        assert summary_line([True, False, True], "x.log", words) == "1 passed, 1 withheld of 3 stages; the output is in x.log"
        assert summary_line([True], "x.log", [("a", PASS_WORD, "")]) == "1 passed, 0 withheld of 1 stages; the output is in x.log"

        # W527 (W524 F-G-5): the checks stages run checks.py --strict, whose NOT RUN code fails the round. The null: before
        # W527 the stages had no --strict, so checks.py skipped a check that could not run and exited with 0.
        for kind in ("plugin", "mge"):
            checks = dict((name, command) for name, _, command in stages_for(kind, os.path.join(root, "nirender-plugin"), []))["checks"]
            assert "--strict" in checks, checks
            # W527 (W524 F-G-6): the round's base reaches the checks, as the round's tree is at the merge.
            on_base = dict((name, command) for name, _, command in stages_for(kind, os.path.join(root, "nirender-plugin"), [], base="abc"))["checks"]
            assert on_base[-2:] == ["--base", "abc"] and "--base" not in checks, (on_base, checks)
        assert NOT_RUN_CODE == NOT_RUN_EXIT, "checks.py's NOT RUN code is the one run_stage reads"
        with open(os.path.join(root, "log5b.txt"), "w", encoding="utf-8") as log:
            not_run = "checks (plugin): 3 passed, 1 NOT RUN (--strict; not on this machine: planted)"
            assert not run_stage("checks", root, [sys.executable, "-c", "print(%r); raise SystemExit(%d)" % (not_run, NOT_RUN_CODE)], log)

        # W497: a stage whose MWSE checks read the sibling MWSE says so in its line, passed or failed.
        fell_back = "MWSE FALLBACK: mwse native: read the sibling checkout x: no single MWSE record in the installs: A none"
        with open(os.path.join(root, "log6.txt"), "w", encoding="utf-8") as log:
            assert run_stage("checks", root, [sys.executable, "-c", "print(%r)" % fell_back], log)
            assert not run_stage("checks", root, [sys.executable, "-c", "print(%r); raise SystemExit(1)" % fell_back], log)
        with open(os.path.join(root, "log6.txt"), encoding="utf-8") as log:
            text = log.read()
        note = "MWSE FALLBACK in mwse native (the sibling MWSE was read, not the installs' commit)"
        assert re.search(r"== PASS checks \(\d+ s\); " + re.escape(note) + "\n", text), text
        assert re.search(r"== FAIL checks \(\d+ s\): exit code 1; " + re.escape(note) + "\n", text), text

        # A replay that the slot check refused: the FAIL line names the reason, as validate_retained prints it (indented,
        # the last lines of replay.py). An older refusal in the log before the stage does not count.
        refused = "       Refused by the slot check: replay.py: only 29.0 GB free on C, and a build needs 30 GB. Ask the Coordinator to read the lists"
        with open(os.path.join(root, "log5.txt"), "w", encoding="utf-8") as log:
            assert not run_stage("replay gate", root, [sys.executable, "-c", "print(%r); raise SystemExit(1)" % refused], log)
            assert not run_stage("after", root, [sys.executable, "-c", "raise SystemExit(4)"], log)
        with open(os.path.join(root, "log5.txt"), encoding="utf-8") as log:
            text = log.read()
        assert ": exit code 1; refused by the slot check: only 29.0 GB free on C, and a build needs 30 GB\n" in text, text
        assert "== FAIL after (" in text and text.count("refused by the slot check") == 1, text
        assert slot_refusal_in("x\nRefused by the slot check: build tool: the build slot is held by A: x, since t. More") == "the build slot is held by A: x, since t"
        assert slot_refusal_in("Refused by the slot check: game: x") == "game: x" and slot_refusal_in("FAIL x") is None

        # The release prints every line of the reply: the free space on C and the warning follow the first line.
        class Released:
            @staticmethod
            def release(slot, who):
                return types.SimpleNamespace(code=0, text="%s: released: x\ndisk C: 29.0 GB free\nWARNING: below 30 GB" % slot)

        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            release_slot(Released, "build", "x")
        assert shown.getvalue() == "build: released: x\ndisk C: 29.0 GB free\nWARNING: below 30 GB\n", shown.getvalue()

        # W567 part B: the round stops at the first stage that does not pass; each later stage is NOT RUN with its name, and
        # does not start. A fault is exit code 1: exit code 4 of a set stage is the maker's MISSING FEATURE.
        good, bad = [sys.executable, "-c", "pass"], [sys.executable, "-c", "raise SystemExit(1)"]
        never = [sys.executable, "-c", "raise SystemExit(9)"]
        now = os.path.join(root, "now")
        gate_stage = ("replay gate", root, good + ["--now", now])
        os.makedirs(os.path.join(now, "tour"))
        stop = "did not pass, and the round stops at its first failure"
        with open(os.path.join(root, "log2.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, bad), (SET_STAGE, root, never), gate_stage], log) == [False, False, False]
            assert [word for _, word, _ in STAGE_WORDS] == [FAIL_WORD, NOT_RUN_WORD, NOT_RUN_WORD], STAGE_WORDS
            assert run_stages([("a", root, good), (SET_STAGE, root, bad), gate_stage], log) == [True, False, False]
        with open(os.path.join(root, "log2.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "exit code 9" not in text, "a stage ran after the first failure"
        assert "== NOT RUN %s: a %s" % (SET_STAGE, stop) in text and "== NOT RUN replay gate: a %s" % stop in text, text
        assert "== replay gate:" not in text and "== NOT RUN replay gate: %s %s" % (SET_STAGE, stop) in text, text
        # The null: the same stages with no failure all run.
        with open(os.path.join(root, "log2b.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, good), (SET_STAGE, root, good), ("b", root, good)], log) == [True, True, True]
        # The fast stages come first in every round (W567 part B): no stage of a set, a deploy or a gate before them.
        for kind, tree, paths in (("plugin", plugin, ["host/src/material.rs"]), ("mge", mge, ["mgeHost64/src/renderer.rs"])):
            names = [name for name, _, _ in stages_for(kind, tree, paths)]
            fast = [name for name, _, _ in fast_stages(kind, plugin, mge)]
            assert names[:len(fast)] == fast and SET_STAGE in names, (kind, names)
        # A stale picture cannot compare: the gate clears its folder and the tour's.
        for stale in (os.path.join(now, "old.png"), os.path.join(now, "tour", "old.png")):
            open(stale, "w", encoding="ascii").close()
        with open(os.path.join(root, "log3.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, good), (SET_STAGE, root, good), gate_stage], log) == [True, True, True]
        assert os.listdir(now) == ["tour"] and os.listdir(os.path.join(now, "tour")) == []

        # The deploy of the set (D193) stands between the set and the gate. It runs when every stage before it passed,
        # whatever the gate says, as the gate comes after it; a failed code stage or deploy stops the round.
        deploy_stage = (DEPLOY_STAGE, root, good)
        with open(os.path.join(root, "log5.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, good), (SET_STAGE, root, good), deploy_stage, ("replay gate", root, bad + ["--now", now])], log) == [True, True, True, False]
            assert run_stages([("a", root, good), (SET_STAGE, root, good), (DEPLOY_STAGE, root, bad), gate_stage], log) == [True, True, False, False]
            assert run_stages([("a", root, bad), (SET_STAGE, root, good), deploy_stage, gate_stage], log) == [False, False, False, False]
            assert run_stages([("a", root, good), (SET_STAGE, root, bad), deploy_stage, gate_stage], log) == [True, False, False, False]
        with open(os.path.join(root, "log5.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== NOT RUN deploy rest: a %s" % stop in text and "== NOT RUN deploy rest: %s %s" % (SET_STAGE, stop) in text, text
        assert "== NOT RUN replay gate: deploy rest %s" % stop in text, text
        # A red gate comes after the deploy, so the deploy runs beside it (the owner tests the latest build, D193); the stages
        # after the gate stop (W567 part B; the Coordinator accepted it: with part D a gate FAIL is a real fault).
        gate_bad = ("replay gate", root, bad + ["--now", now])
        with open(os.path.join(root, "log6.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, good), (SET_STAGE, root, good), deploy_stage, gate_bad, (BASE_STAGE, root, never), (INSTANCE_STAGE, root, never)], log) == [True, True, True, False, False, False]
        with open(os.path.join(root, "log6.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "PASS deploy rest" in text, "the deploy did not run beside a red gate"
        assert "== NOT RUN %s: replay gate %s" % (BASE_STAGE, stop) in text and "== NOT RUN %s: replay gate %s" % (INSTANCE_STAGE, stop) in text, text

        # A refused deploy (a game or Mod Organizer runs) is no failure of the round: the line "WITHHELD deploy rest" names the command
        # that deploys alone. A fault of the deploy, and the exit code of the refusal with no "REFUSED" line, still fail.
        refused = [sys.executable, "-c", "print('REFUSED: Morrowind.exe runs, and the deploy does not wait: end it'); raise SystemExit(%d)" % DEPLOY_REFUSED_CODE]
        with open(os.path.join(root, "log7.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, good), (SET_STAGE, root, good), (DEPLOY_STAGE, root, refused), gate_stage], log) == [True, True, True, True]
            # W528: each stage has its word, and the close names the WITHHELD stage with its reason.
            assert [word for _, word, _ in STAGE_WORDS] == [PASS_WORD, PASS_WORD, WITHHELD_WORD, PASS_WORD], STAGE_WORDS
            column = stages_column(STAGE_WORDS)
            assert column == ("PASS 3, WITHHELD 1 of 4: a, %s, replay gate; WITHHELD deploy rest: Morrowind.exe runs, "
                              "and the deploy does not wait: end it" % SET_STAGE), column
            assert len(NOT_DEPLOYED) == 1 and "Morrowind.exe runs" in NOT_DEPLOYED[0] and "To deploy this build alone, run in %s: " % root in NOT_DEPLOYED[0] and subprocess.list2cmdline(refused) in NOT_DEPLOYED[0], NOT_DEPLOYED
            # A folder and an argument with a space are quoted: the line pastes into a shell.
            spaced = os.path.join(root, "a folder")
            os.makedirs(spaced)
            assert run_stages([("a", root, good), (DEPLOY_STAGE, spaced, refused)], log) == [True, True]
            assert 'run in "%s": ' % spaced in NOT_DEPLOYED[0] and '"print(' in NOT_DEPLOYED[0], NOT_DEPLOYED
            assert run_stages([("a", root, good), (DEPLOY_STAGE, root, [sys.executable, "-c", "print('REFUSED: x'); raise SystemExit(1)"])], log) == [True, False], "another code is a fault"
            assert NOT_DEPLOYED == [], "run_stages starts a new list"
            assert run_stages([("a", root, good), (DEPLOY_STAGE, root, [sys.executable, "-c", "print('FAIL: x'); raise SystemExit(%d)" % DEPLOY_REFUSED_CODE])], log) == [True, False], "the code with no REFUSED line is a fault"
            assert run_stages([("a", root, good), ("docs", root, refused)], log) == [True, False], "another stage is not excused by the refusal"
        with open(os.path.join(root, "log7.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== WITHHELD deploy rest (" in text and "PASS replay gate" in text, text

        gate_names = [name for name, _, _ in stages_for("plugin", plugin, ["host/src/material.rs"])]
        assert gate_names.index(SET_STAGE) + 1 == gate_names.index(DEPLOY_STAGE) == gate_names.index("replay gate") - 1
        assert DEPLOY_STAGE not in [name for name, _, _ in stages_for("mge", mge, ["xtask/src/main.rs"])], "a build of the dist alone is not deployed"
        assert DEPLOY_STAGE not in [name for name, _, _ in stages_for("plugin", plugin, ["tools/x.py"])]

        # Q10 (addendum 10 C4, test 3): the maker's MISSING FEATURE (exit code 4) makes the set stage WITHHELD, and the stages
        # that take the set are NOT RUN, so the round fails; the line names the missing feature and --mwse. The null: a set
        # that was made lets them run. Exit code 4 of another stage is a fault.
        maker_missing = [sys.executable, "-c", "print('probe: MISSING: count locks: 0 of 40 count lock sites: the plugin needs it'); "
                         "print('MISSING FEATURE: the MWSE.dll of the set lacks what the plugin needs; no set is made'); raise SystemExit(%d)" % MISSING_FEATURE_EXIT,
                         "--mwse", "e" * 40]
        with open(os.path.join(root, "log8.txt"), "w", encoding="utf-8") as log:
            stages = [("a", root, good), (SET_STAGE, root, maker_missing), (SITES_SET_STAGE, root, never), (DEPLOY_STAGE, root, never), gate_stage,
                      (HELPER_SOURCES_STAGE, root, never), (BASE_STAGE, root, good), (INSTANCE_STAGE, root, never)]
            assert run_stages(stages, log) == [True, True, False, False, False, False, False, False]
            assert [word for _, word, _ in STAGE_WORDS] == [PASS_WORD, WITHHELD_WORD] + [NOT_RUN_WORD] * 6, STAGE_WORDS
            reason = STAGE_WORDS[1][2]
            assert "MISSING count locks: 0 of 40" in reason and "--mwse <an MWSE commit that has it>" in reason and "MWSE eeeeeeeeee" in reason, reason
            assert NOT_DEPLOYED == []
            # The base set's MISSING FEATURE stops the A/B alone.
            assert run_stages([(SET_STAGE, root, good), (BASE_STAGE, root, maker_missing), (INSTANCE_STAGE, root, never)], log) == [True, True, False]
            assert run_stages([("docs", root, maker_missing)], log) == [False], "exit code 4 of another stage is a fault"
        with open(os.path.join(root, "log8.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== %s: " % DEPLOY_STAGE not in text and "== replay gate: " not in text, "a stage took a set that was not made"
        assert "== WITHHELD %s (" % SET_STAGE in text and "== NOT RUN %s: the round's set was not made" % SITES_SET_STAGE in text, text
        assert "== NOT RUN deploy rest: %s %s" % (SITES_SET_STAGE, stop) in text, text
        assert maker_missing_in("probe: MISSING: x: 0 of 1\nprobe: feature: y: present\nMISSING: z\n") == ["x: 0 of 1"]
        # The deploy of the round needs the game slot: its set on E:, its refusal while a game runs.
        assert needs_game(stages_for("plugin", plugin, ["src/lib.rs"])) and needs_game(stages_for("plugin", plugin, ["mod/MWSE/mods/nirender/main.lua"]))
        assert not needs_game(stages_for("plugin", plugin, ["tools/x.py"])) and not needs_game(stages_for("mge", mge, ["d3d8/x.cpp"]))
        with_set = stages_for("plugin", plugin, ["src/lib.rs"], commits=full, mwse_commit=mwse)
        assert any(EVIDENCE_PATH.search(part) for _, _, command in with_set for part in command), "the set's folder is on E:"
        assert evidence_problem(with_set) is None, "the round holds the game slot, so its stages may name E:"
        deploy = next(stage for stage in stages_for("plugin", plugin, ["host/src/material.rs"]) if stage[0] == DEPLOY_STAGE)
        assert deploy[1] == plugin and deploy[2][deploy[2].index("--set") + 1] == UNPLANNED_SET, deploy
        assert deploy_refusal_in("x\nREFUSED: Morrowind.exe runs, and the deploy does not wait: end it\nFAIL: y") == "Morrowind.exe runs, and the deploy does not wait: end it"
        assert deploy_refusal_in("FAIL: x") is None

        # Each stage that runs notes the slot's work, and a gate that does not run does not.
        notes = []
        with open(os.path.join(root, "log4.txt"), "w", encoding="utf-8") as log:
            run_stages([("a", root, good), (SET_STAGE, root, bad), gate_stage], log, lambda: notes.append(1))
        assert len(notes) == 2

        # The A/B stages follow the gate, with the base pair of both repos, and only with it (addendum 10 C3, test 4): the
        # base set of the pair and the round's MWSE commit, made in the round's take.
        tips = {"plugin": "1" * 40, "mge": "2" * 40}
        with_tips = stages_for("plugin", plugin, ["host/src/material.rs"], BASELINE, tips, mwse, commits=full, label="merge_step: x")
        assert [name for name, _, _ in with_tips][-6:] == [SET_STAGE, DEPLOY_STAGE, "replay gate", HELPER_SOURCES_STAGE, BASE_STAGE, INSTANCE_STAGE]
        by_name = {name: (folder, command) for name, folder, command in with_tips}
        build = by_name[BASE_STAGE][1]
        assert build[build.index("--nirender") + 1] == "1" * 40 and build[build.index("--render-link") + 1] == "2" * 40 and build[build.index("--mwse") + 1] == mwse, build
        assert build[build.index(IN_HOLD) + 1] == "merge_step: x", build
        run = by_name[INSTANCE_STAGE][1]
        assert run[run.index("--base-build") + 1] == os.path.join(sets.SETS, "s-11111111-22222222-cccccccc") and run[run.index("--now-build") + 1] == the_set, run
        assert run[run.index("--baseline") + 1] == BASELINE and not run[run.index("--out") + 1].startswith(BASELINE)
        assert [name for name, _, _ in stages_for("plugin", plugin, ["src/lib.rs"], BASELINE, tips, mwse)] == base + deploy_tail, "no gate, no A/B stage"
        assert [name for name, _, _ in stages_for("plugin", plugin, ["host/src/material.rs"], BASELINE)][-1] == HELPER_SOURCES_STAGE, "no base pair, no A/B stage"
        # deploy rest gets the commits under test (the tree's and the sibling's at the start of the round), not HEADs.
        assert under_test("plugin", "a" * 40, "b" * 40) == {"plugin": "a" * 40, "mge": "b" * 40}
        assert under_test("mge", "b" * 40, "a" * 40) == {"plugin": "a" * 40, "mge": "b" * 40}
        assert under_test("plugin", "a" * 40, None) is None and under_test("", "a", "b") is None
        deploy = by_name[DEPLOY_STAGE][1]
        assert deploy[deploy.index("--plugin-commit") + 1] == "a" * 40 and deploy[deploy.index("--mge-commit") + 1] == "b" * 40, deploy
        assert "--plugin-commit" not in next(command for name, _, command in stages_for("plugin", plugin, ["host/src/material.rs"]) if name == DEPLOY_STAGE)
        assert tips_for("plugin", os.path.join(root, "alone", PLUGIN_NAME), "b")[0] is None
        # The sibling's side of the base is the pair that passed with the base, kept in the sibling's repo.
        pair = os.path.join(root, "pair")
        sibling_tree = os.path.join(pair, MGE_NAME)
        os.makedirs(sibling_tree)
        author = ["-c", "user.name=t", "-c", "user.email=t@example.com"]
        for command in (["init", "-q"], author + ["commit", "-q", "--allow-empty", "-m", "one"], author + ["commit", "-q", "--allow-empty", "-m", "two"]):
            subprocess.run(["git", "-C", sibling_tree, *command], check=True, env=git_env())
        second = resolve(sibling_tree, "HEAD")
        subprocess.run(["git", "-C", sibling_tree, "checkout", "-q", "--detach", "HEAD~1"], check=True, env=git_env())
        first = resolve(sibling_tree, "HEAD")
        assert first != second
        plugin_of_pair = os.path.join(pair, PLUGIN_NAME)
        no_pair = tips_for("plugin", plugin_of_pair, "base")
        assert no_pair[0] is None and "keeps no pair for the base" in no_pair[1] and "--sibling-base" in no_pair[1], "no kept pair, no option: no base set"
        keep_pair("plugin", sibling_tree, "base", first)
        assert resolve(sibling_tree, pair_ref("plugin", "base")) == first
        assert tips_for("plugin", plugin_of_pair, "base") == ({"plugin": "base", "mge": first}, None), "the kept pair, HEAD itself"
        assert tips_for("mge", sibling_tree, "base")[1].startswith("no sibling")
        # --sibling-base: the sibling's side of the base is that commit, when it is HEAD or an ancestor of HEAD.
        assert tips_for("plugin", plugin_of_pair, "base", first) == ({"plugin": "base", "mge": first}, None), "HEAD itself"
        subprocess.run(["git", "-C", sibling_tree, "checkout", "-q", "--detach", second], check=True, env=git_env())
        subprocess.run(["git", "-C", sibling_tree, *author, "commit", "-q", "--allow-empty", "-m", "three"], check=True, env=git_env())
        third = resolve(sibling_tree, "HEAD")
        # The fault of 2026-11-10: the sibling's HEAD moved on after the pass of the base; the base stays at the kept pair.
        assert tips_for("plugin", plugin_of_pair, "base") == ({"plugin": "base", "mge": first}, None), "without the option: the kept pair, not the sibling's HEAD"
        assert tips_for("plugin", plugin_of_pair, "base", second) == ({"plugin": "base", "mge": second}, None), "an ancestor of HEAD, in place of the kept pair"
        assert tips_for("plugin", plugin_of_pair, "base", second[:10]) == ({"plugin": "base", "mge": second}, None), "a short name gives the full commit"
        subprocess.run(["git", "-C", sibling_tree, "checkout", "-q", "--detach", first], check=True, env=git_env())
        assert tips_for("plugin", plugin_of_pair, "base", second)[1].endswith("(%s)" % first[:10]) and "not an ancestor" in tips_for("plugin", plugin_of_pair, "base", second)[1], "a commit ahead of HEAD"
        keep_pair("plugin", sibling_tree, "later", second)
        assert tips_for("plugin", plugin_of_pair, "later")[1].startswith("the kept pair %s is not an ancestor" % pair_ref("plugin", "later")), "a kept pair ahead of HEAD"
        # keep_pass, which a pass and a round with the A/B base older than its capture share: the ref of the branch moves to
        # the commit, and the pair is kept when the sibling's HEAD did not move; else no pair.
        with contextlib.redirect_stdout(io.StringIO()):
            keep_pass("plugin", sibling_tree, "refs/merge-step/test", second, sibling_tree, first)
        assert resolve(sibling_tree, "refs/merge-step/test") == second and resolve(sibling_tree, pair_ref("plugin", second)) == first
        with contextlib.redirect_stdout(io.StringIO()) as said:
            keep_pass("plugin", sibling_tree, "refs/merge-step/test", third, sibling_tree, second)
        assert resolve(sibling_tree, "refs/merge-step/test") == third and resolve(sibling_tree, pair_ref("plugin", third)) is None and "this round's pair: none kept: " in said.getvalue()
        assert "is no commit" in tips_for("plugin", plugin_of_pair, "base", "no-such-name")[1], "an unknown name"
        # W370: a stage covers another by folder and command, with a pass; the name is no proof.
        one = ("checks", os.path.join(root, "a"), ["python", "x"])
        assert missing_stages([one], [one], [True]) == [], "the same stage, passed"
        assert missing_stages([one], [one], [False]) == ["checks"], "the same stage, failed"
        assert missing_stages([one], [], []) == ["checks"], "no stage ran"
        assert missing_stages([one], [("other name", os.path.join(root, "a", ".").upper(), ["python", "x"])], [True]) == [], "a folder written another way"
        assert missing_stages([one], [("checks", os.path.join(root, "a"), ["python", "y"])], [True]) == ["checks"], "the same name, another command"
        assert missing_stages([one], [("checks", os.path.join(root, "b"), ["python", "x"])], [True]) == ["checks"], "the same name, another folder"
        # W370: the sibling's ref after a pass. A scratch pair: the render-link side has its ref at c1 and HEAD at c2,
        # which changes a path of the helper.
        coverage = os.path.join(root, "coverage")
        cover_plugin, cover_mge = os.path.join(coverage, PLUGIN_NAME), os.path.join(coverage, MGE_NAME)
        for tree, branch in ((cover_plugin, "master"), (cover_mge, "feature/x")):
            os.makedirs(tree)
            subprocess.run(["git", "-C", tree, "init", "-q", "-b", branch], check=True, env=git_env())
            subprocess.run(["git", "-C", tree, *author, "commit", "-q", "--allow-empty", "-m", "c1"], check=True, env=git_env())
        c1 = resolve(cover_mge, "HEAD")
        os.makedirs(os.path.join(cover_mge, "mgeHost64", "src"))
        with open(os.path.join(cover_mge, "mgeHost64", "src", "x.rs"), "w", encoding="ascii") as changed:
            changed.write("x\n")
        subprocess.run(["git", "-C", cover_mge, "add", "mgeHost64/src/x.rs"], check=True, env=git_env())
        subprocess.run(["git", "-C", cover_mge, *author, "commit", "-q", "-m", "c2"], check=True, env=git_env())
        c2 = resolve(cover_mge, "HEAD")
        cover_ref = "refs/merge-step/feature/x"
        cover_commits = {"plugin": resolve(cover_plugin, "HEAD"), "mge": c2}
        # The render-link round from the ref c1: its checks compare with c1 (W527), as the sibling's own check would.
        mge_round = stages_for("mge", cover_mge, ["mgeHost64/src/x.rs"], BASELINE, commits=cover_commits, base=c1)
        plugin_round = stages_for("plugin", cover_plugin, ["tools/x.py"], BASELINE, commits=cover_commits)

        def sibling_move(stages, results, sibling_head=c2, changed=False, mwse_commit="c" * 40):
            with contextlib.redirect_stdout(io.StringIO()) as said:
                moved = move_sibling_ref("plugin", cover_mge, sibling_head, stages, results, BASELINE, cover_commits, changed, mwse_commit=mwse_commit)
            return moved, said.getvalue()

        moved, said = sibling_move(plugin_round, [True] * len(plugin_round))
        assert not moved and "stays absent" in said, said
        subprocess.run(["git", "-C", cover_mge, "update-ref", cover_ref, c1], check=True, env=git_env())
        # The null: a pass of a nirender round runs none of the helper's stages, so the render-link ref stays.
        moved, said = sibling_move(plugin_round, [True] * len(plugin_round))
        assert not moved and resolve(cover_mge, cover_ref) == c1, said
        assert "needs helper tests, helper GPU tests, clippy baseline, checks" in said and "stays at %s" % c1[:10] in said, said
        moved, said = sibling_move(mge_round, [True] * (len(mge_round) - 1) + [False])
        assert not moved and resolve(cover_mge, cover_ref) == c1 and said.rstrip().endswith("check it on its own"), "a failed stage of the diff"
        moved, said = sibling_move(mge_round, [True] * len(mge_round), sibling_head=c1)
        assert not moved and "moved during the round" in said and resolve(cover_mge, cover_ref) == c1, said
        both_round, all_passed = plugin_round + mge_round, [True] * (len(plugin_round) + len(mge_round))
        # Verdict 227: tracked changes of the sibling, at the start of the round or now, keep the ref.
        moved, said = sibling_move(both_round, all_passed, changed=True)
        assert not moved and "tracked changes at the start of the round" in said and resolve(cover_mge, cover_ref) == c1, said
        tracked = os.path.join(cover_mge, "mgeHost64", "src", "x.rs")
        with open(tracked, "w", encoding="ascii") as changed_file:
            changed_file.write("changed\n")
        moved, said = sibling_move(both_round, all_passed)
        assert not moved and "tracked changes now" in said and resolve(cover_mge, cover_ref) == c1, said
        with open(tracked, "w", encoding="ascii") as changed_file:
            changed_file.write("x\n")
        # Verdict 229: git status of the sibling fails at the move (a corrupt index): one line, no SystemExit.
        index = os.path.join(cover_mge, ".git", "index")
        with open(index, "rb") as kept_index:
            index_bytes = kept_index.read()
        with open(index, "wb") as broken_index:
            broken_index.write(b"not an index")
        try:
            moved, said = sibling_move(both_round, all_passed)
        finally:
            with open(index, "wb") as restored_index:
                restored_index.write(index_bytes)
        assert not moved and "git status of the sibling failed" in said and resolve(cover_mge, cover_ref) == c1, said
        # Verdict 227: another check moves the ref between the read and the update. git refuses the update, and the
        # tool gives one line and keeps the other check's value; before, git() ended the round with SystemExit.
        real_planned = planned_stages

        def planned_with_a_race(*args, **kwargs):
            subprocess.run(["git", "-C", cover_mge, "update-ref", cover_ref, c2], check=True, env=git_env())
            return real_planned(*args, **kwargs)

        globals()["planned_stages"] = planned_with_a_race
        try:
            moved, said = sibling_move(both_round, all_passed)
        finally:
            globals()["planned_stages"] = real_planned
        assert not moved and "git update-ref refused the move from %s to %s" % (c1[:10], c2[:10]) in said, said
        assert resolve(cover_mge, cover_ref) == c2 and "stays at %s" % c2[:10] in said, said
        subprocess.run(["git", "-C", cover_mge, "update-ref", cover_ref, c1], check=True, env=git_env())
        # The stages of the sibling's diff all ran and passed: the ref moves to the HEAD of the round.
        moved, said = sibling_move(both_round, all_passed)
        assert moved and resolve(cover_mge, cover_ref) == c2 and "moves from %s to %s" % (c1[:10], c2[:10]) in said, said
        moved, said = sibling_move(mge_round, [True] * len(mge_round))
        assert not moved and "nothing to move" in said, said
        # A ref that is not an ancestor of HEAD, a diff with the gate and no kept pair, and a tree on no branch: it stays.
        subprocess.run(["git", "-C", cover_mge, *author, "commit", "-q", "--allow-empty", "-m", "c3"], check=True, env=git_env())
        c3 = resolve(cover_mge, "HEAD")
        subprocess.run(["git", "-C", cover_mge, "update-ref", cover_ref, c3], check=True, env=git_env())
        subprocess.run(["git", "-C", cover_mge, "reset", "-q", "--soft", c2], check=True, env=git_env())
        moved, said = sibling_move(mge_round, [True] * len(mge_round))
        assert not moved and "not an ancestor" in said and resolve(cover_mge, cover_ref) == c3, said
        subprocess.run(["git", "-C", cover_mge, "update-ref", cover_ref, c1], check=True, env=git_env())
        gate_file = GATE_PATHS["mge"][0]
        assert gate_file == "mgeHost64/src/renderer.rs", "the case writes a file of the gate in a folder that exists"
        with open(os.path.join(cover_mge, gate_file), "w", encoding="ascii") as changed:
            changed.write("x\n")
        subprocess.run(["git", "-C", cover_mge, "add", gate_file], check=True, env=git_env())
        subprocess.run(["git", "-C", cover_mge, *author, "commit", "-q", "-m", "c4"], check=True, env=git_env())
        c4 = resolve(cover_mge, "HEAD")
        moved, said = sibling_move(mge_round, [True] * len(mge_round), sibling_head=c4)
        assert not moved and "are not known" in said and "keeps no pair" in said and resolve(cover_mge, cover_ref) == c1, said
        # The sibling's diff makes a set: with no MWSE commit of the round its stages are not known either.
        moved, said = sibling_move(mge_round, [True] * len(mge_round), sibling_head=c4, mwse_commit=None)
        assert not moved and "are not known" in said and "give --mwse" in said and resolve(cover_mge, cover_ref) == c1, said
        # The round's MWSE commit: --mwse resolved to the full commit, else the installs' records.
        assert round_mwse(c1[:8], cover_mge) == (c1, None) and round_mwse("no-such-name", cover_mge) == (None, "--mwse no-such-name is no commit of %s" % cover_mge)
        assert round_mwse(None, cover_mge, lambda: ("f" * 40, None)) == ("f" * 40, None) and round_mwse(None, cover_mge, lambda: (None, "why"))[1] == "why"
        subprocess.run(["git", "-C", cover_mge, "checkout", "-q", "--detach", "HEAD"], check=True, env=git_env())
        moved, said = sibling_move(mge_round, [True] * len(mge_round), sibling_head=c4)
        assert not moved and "on no branch" in said, said
        # resolve gives a commit: a tree is none, and an annotated tag gives the commit it points to.
        tree_of_head = subprocess.run(["git", "-C", sibling_tree, "rev-parse", "HEAD^{tree}"], capture_output=True, text=True, env=git_env()).stdout.strip()
        assert len(tree_of_head) == 40 and resolve(sibling_tree, tree_of_head) is None, "the name of a tree is no commit"
        assert "is no commit" in tips_for("plugin", plugin_of_pair, "base", tree_of_head)[1], "--sibling-base with a tree"
        subprocess.run(["git", "-C", sibling_tree, *author, "tag", "-a", "-m", "a tag", "a-tag", first], check=True, env=git_env())
        assert resolve(sibling_tree, "a-tag") == first, "an annotated tag gives its commit, not the tag object"
        assert tips_for("plugin", plugin_of_pair, "base", "a-tag") == ({"plugin": "base", "mge": first}, None)
        # git() and branch_of answer for the folder they get, also with the GIT_ names of a hook in the environment.
        alpha, beta = os.path.join(root, "alpha"), os.path.join(root, "beta")
        for folder, branch in ((alpha, "alpha-branch"), (beta, "beta-branch")):
            os.makedirs(folder)
            for command in (["init", "-q", "-b", branch], author + ["commit", "-q", "--allow-empty", "-m", branch]):
                subprocess.run(["git", "-C", folder, *command], check=True, env=git_env())
        beta_head = resolve(beta, "HEAD")
        os.environ["GIT_DIR"] = os.path.join(alpha, ".git")
        try:
            assert branch_of(beta) == "beta-branch", "branch_of answers for the hook's repository: %s" % branch_of(beta)
            assert git(beta, "rev-parse", "HEAD") == beta_head, "git() answers for the hook's repository"
        finally:
            del os.environ["GIT_DIR"]
        # W528: a file moved out of a gate path plans the gate; a move inside tools/ plans none.
        moves = os.path.join(root, "moves")
        os.makedirs(os.path.join(moves, "mgeHost64", "src"))
        os.makedirs(os.path.join(moves, "tools"))
        for name in ("mgeHost64/src/renderer.rs", "tools/a.py"):
            with open(os.path.join(moves, name), "w", encoding="ascii") as moved_file:
                moved_file.write("a file with enough lines\nto be found as a move\nby git diff\n")
        subprocess.run(["git", "-C", moves, "init", "-q"], check=True, env=git_env())
        subprocess.run(["git", "-C", moves, "add", "."], check=True, env=git_env())
        subprocess.run(["git", "-C", moves, *author, "commit", "-q", "-m", "m1"], check=True, env=git_env())
        before_moves = resolve(moves, "HEAD")
        subprocess.run(["git", "-C", moves, "mv", "mgeHost64/src/renderer.rs", "tools/renderer.rs"], check=True, env=git_env())
        subprocess.run(["git", "-C", moves, *author, "commit", "-q", "-m", "m2"], check=True, env=git_env())
        out_of_gate = resolve(moves, "HEAD")
        subprocess.run(["git", "-C", moves, "mv", "tools/a.py", "tools/b.py"], check=True, env=git_env())
        subprocess.run(["git", "-C", moves, *author, "commit", "-q", "-m", "m3"], check=True, env=git_env())
        inside_tools = resolve(moves, "HEAD")
        renamed = git(moves, "-c", "diff.renames=true", "diff", "--name-only", before_moves, out_of_gate).splitlines()
        assert renamed == ["tools/renderer.rs"], "the case is a move that git finds: %s" % renamed
        assert not touches(renamed, GATE_PATHS["mge"]), "the null of the plant: the new path alone plans no gate"
        both = changed_paths(moves, before_moves, out_of_gate)
        assert sorted(both) == ["mgeHost64/src/renderer.rs", "tools/renderer.rs"], both
        assert touches(both, GATE_PATHS["mge"]), "a move out of a gate path plans the gate"
        inside = changed_paths(moves, out_of_gate, inside_tools)
        assert sorted(inside) == ["tools/a.py", "tools/b.py"] and not touches(inside, GATE_PATHS["mge"]), inside
        # W528 item 1: the helper's sources from the dep-info file of its build plan the gate.
        source_roots = {"plugin": os.path.join(root, "src-plugin"), "mge": os.path.join(root, "src mge")}
        dep_text = "%s: %s %s %s %s %s %s\n\n%s:\n" % (
            os.path.join(source_roots["mge"], "target", "numidium.exe").replace(" ", "\\ "),
            os.path.join(source_roots["plugin"], "src", "stream_format.rs"),
            os.path.join(source_roots["mge"], "mgeHost64", "src", "main.rs").replace(" ", "\\ "),
            os.path.join(source_roots["mge"], "target", "out", "gen.rs").replace(" ", "\\ "),
            os.path.join(source_roots["plugin"], ".git", "HEAD"),
            os.path.join(root, "elsewhere", "lib.rs"),
            os.path.join(source_roots["plugin"], "host", "src", "lib.rs"),
            os.path.join(source_roots["plugin"], "src", "stream_format.rs"))
        listed = dep_info_paths(dep_text)
        assert len(listed) == 6 and listed[1] == os.path.join(source_roots["mge"], "mgeHost64", "src", "main.rs"), listed
        assert sources_by_repo(listed, source_roots) == {"plugin": ["host/src/lib.rs", "src/stream_format.rs"], "mge": ["mgeHost64/src/main.rs"]}
        assert dep_info_paths("no rule here\n") == []
        dep_file, list_file = os.path.join(root, "numidium.d"), os.path.join(root, "keep", "helper-sources.txt")
        with open(dep_file, "w", encoding="utf-8") as file:
            file.write(dep_text)
        with contextlib.redirect_stdout(io.StringIO()) as said:
            assert keep_helper_sources(dep_file, list_file, source_roots["plugin"], source_roots["mge"], before_moves, before_moves) == 0
        assert said.getvalue().startswith("helper sources: 2 of nirender-plugin and 1 of render-link"), said.getvalue()
        assert read_helper_sources(list_file) == ({"plugin": before_moves, "mge": before_moves},
                                                  {"plugin": ("host/src/lib.rs", "src/stream_format.rs"), "mge": ("mgeHost64/src/main.rs",)})
        # A missing dep-info file, and one that names no source of a repo, fail the stage and keep the old list.
        with contextlib.redirect_stdout(io.StringIO()) as said:
            assert keep_helper_sources(os.path.join(root, "none.d"), list_file, source_roots["plugin"], source_roots["mge"], "a", "b") == 1
            assert keep_helper_sources(dep_file, list_file, source_roots["plugin"], os.path.join(root, "other"), "a", "b") == 1
        assert "FAIL: no dep-info file" in said.getvalue() and "names no source of %s" % os.path.join(root, "other") in said.getvalue()
        assert read_helper_sources(list_file)[0]["plugin"] == before_moves, "a failed stage wrote the list"
        # Addendum 10 C5 (test 5): the list is read only when the numidium.exe beside the dep-info file is the set's. A part
        # that the maker copied from another set passes the stage with a line and keeps the list; the null: the same
        # numidium.exe writes it.
        set_list = os.path.join(root, "keep", "set-sources.txt")
        built_exe, set_exe = os.path.join(root, "numidium.exe"), os.path.join(root, "set-numidium.exe")
        for path, body in ((built_exe, b"built in the tree"), (set_exe, b"copied from another set")):
            with open(path, "wb") as file:
                file.write(body)
        with contextlib.redirect_stdout(io.StringIO()) as said:
            assert keep_helper_sources(dep_file, set_list, source_roots["plugin"], source_roots["mge"], "a", "b", set_exe) == 0
        assert not os.path.exists(set_list) and "is not the build of the maker's tree" in said.getvalue(), said.getvalue()
        with open(set_exe, "wb") as file:
            file.write(b"built in the tree")
        with contextlib.redirect_stdout(io.StringIO()):
            assert keep_helper_sources(dep_file, set_list, source_roots["plugin"], source_roots["mge"], "a", "b", set_exe) == 0
        assert read_helper_sources(set_list)[0] == {"plugin": "a", "mge": "b"}, read_helper_sources(set_list)
        # The plant: a list of an ancestor of the base; a change of a listed file plans the gate, and the line names the list.
        helper_paths, line = helper_gate_paths("plugin", moves, inside_tools, list_file)
        assert helper_paths == ("host/src/lib.rs", "src/stream_format.rs") and before_moves[:10] in line, line
        assert "replay gate" in [name for name, _, _ in stages_for("plugin", plugin, ["src/stream_format.rs"], helper_paths=helper_paths)]
        assert "replay gate" not in [name for name, _, _ in stages_for("plugin", plugin, ["src/stream_format.rs"])], "the null of the plant: GATE_PATHS alone"
        # The null: a change of a file that is not listed plans no gate.
        assert "replay gate" not in [name for name, _, _ in stages_for("plugin", plugin, ["tools/x.py"], helper_paths=helper_paths)]
        # No list, a list not in the form, or a list of a commit that is not an ancestor of the base: the gate, with why.
        assert helper_gate_paths("plugin", moves, inside_tools, os.path.join(root, "no-list.txt"))[0] is None
        with open(os.path.join(root, "bad-list.txt"), "w", encoding="ascii") as file:
            file.write("plugin\tsrc/x.rs\n")
        assert helper_gate_paths("plugin", moves, inside_tools, os.path.join(root, "bad-list.txt"))[0] is None
        assert helper_gate_paths("plugin", moves, before_moves, list_file)[0] is not None, "the list's own commit is the base"
        with contextlib.redirect_stdout(io.StringIO()):
            keep_helper_sources(dep_file, list_file, source_roots["plugin"], source_roots["mge"], inside_tools, inside_tools)
        stale, line = helper_gate_paths("plugin", moves, before_moves, list_file)
        assert stale is None and "is not an ancestor of the base %s" % before_moves[:10] in line, line
        assert "replay gate" in [name for name, _, _ in stages_for("plugin", plugin, ["tools/x.py"], helper_paths=None)], "an unknown list plans the gate"
        # The stage stands after the gate, and is NOT RUN when the round's set was not made.
        with open(os.path.join(root, "log-sources.txt"), "w", encoding="utf-8") as log, contextlib.redirect_stdout(io.StringIO()):
            assert run_stages([(SET_STAGE, root, [sys.executable, "-c", "raise SystemExit(1)"]),
                               (HELPER_SOURCES_STAGE, root, [sys.executable, "-c", "raise SystemExit(9)"])], log) == [False, False]
        assert STAGE_WORDS[-1] == (HELPER_SOURCES_STAGE, NOT_RUN_WORD, "%s did not pass, and the round stops at its first failure" % SET_STAGE), STAGE_WORDS
        with open(os.path.join(root, "log-sources.txt"), "w", encoding="utf-8") as log, contextlib.redirect_stdout(io.StringIO()):
            assert run_stages([(SET_STAGE, root, maker_missing), (HELPER_SOURCES_STAGE, root, never)], log) == [True, False]
        assert STAGE_WORDS[-1] == (HELPER_SOURCES_STAGE, NOT_RUN_WORD, "the round's set was not made"), STAGE_WORDS
        # W528 item 3b: a round with a WITHHELD deploy keeps an open deploy until a deploy of the same or newer commits.
        # The scratch repo of the moves stands for both repos: before_moves, out_of_gate and inside_tools in that order.
        deploys = os.path.join(root, "deploys", "deploys.tsv")
        assert deploys_open(moves, moves, deploys) == [], "no file: none open"
        said = withhold_deploy(deploys, "t1", {"plugin": out_of_gate, "mge": out_of_gate}, ["WITHHELD deploy rest (1 s): a game\truns"])
        assert said.startswith("deploys: WITHHELD plugin %s, mge %s in " % (out_of_gate[:10], out_of_gate[:10])), said
        still = deploys_open(moves, moves, deploys)
        assert still == [("t1", out_of_gate, out_of_gate, "WITHHELD deploy rest (1 s): a game runs")], still
        assert open_deploy_lines(still)[0] == "open deploys: 1 (the oldest t1: plugin %s, mge %s)" % (out_of_gate[:10], out_of_gate[:10])
        closed = close_after([True], BASELINE, [("c" * 40, "Merge x (W1)")], lambda merges, stages: stages,
                             [("a", WITHHELD_WORD, "a game runs")], extra=open_deploy_lines(still)[:1])
        assert closed == "WITHHELD 1 of 1: none; WITHHELD a: a game runs; open deploys: 1 (the oldest t1: plugin %s, mge %s)" % (
            out_of_gate[:10], out_of_gate[:10]), closed
        # A deploy of an older commit, or of a commit that the repo does not know, leaves it open.
        for done_plugin, done_mge in ((before_moves, out_of_gate), (out_of_gate, "f" * 40)):
            append_deploys(deploys, deploys_line("t2", "DEPLOYED", done_plugin, done_mge, "A, B"))
            assert len(deploys_open(moves, moves, deploys)) == 1
        # The null: a deploy of newer commits closes it, and a write to a folder that cannot be made is a line.
        append_deploys(deploys, deploys_line("t3", "DEPLOYED", inside_tools, out_of_gate, "A, B"))
        assert deploys_open(moves, moves, deploys) == [] and open_deploy_lines([]) == ["open deploys: 0"]
        blocked = os.path.join(root, "deploys", "deploys.tsv", "x.tsv")
        assert withhold_deploy(blocked, "t4", None, ["x"]).startswith("deploys: NOT WRITTEN to %s" % blocked)
        # W532: the ref lines of a round are in its log, after the summary. A scratch pair: nirender with its ref at s1
        # and HEAD at s2; render-link with no ref of its own.
        settle = os.path.join(root, "settle")
        settle_plugin, settle_mge = os.path.join(settle, PLUGIN_NAME), os.path.join(settle, MGE_NAME)
        for tree, branch in ((settle_plugin, "master"), (settle_mge, "feature/y")):
            os.makedirs(tree)
            subprocess.run(["git", "-C", tree, "init", "-q", "-b", branch], check=True, env=git_env())
            subprocess.run(["git", "-C", tree, *author, "commit", "-q", "--allow-empty", "-m", "s1"], check=True, env=git_env())
        s1 = resolve(settle_plugin, "HEAD")
        subprocess.run(["git", "-C", settle_plugin, *author, "commit", "-q", "--allow-empty", "-m", "s2"], check=True, env=git_env())
        s2 = resolve(settle_plugin, "HEAD")
        settle_ref = "refs/merge-step/master"
        subprocess.run(["git", "-C", settle_plugin, "update-ref", settle_ref, s1], check=True, env=git_env())
        settle_head = resolve(settle_mge, "HEAD")
        settle_stages = [("checks", settle_plugin, ["python", "x"])]

        def settled(results, baseline=BASELINE):
            log_path = os.path.join(settle, "round-%d.log" % len(os.listdir(settle)))
            with contextlib.redirect_stdout(io.StringIO()):
                code = settle_refs("plugin", settle_plugin, settle_ref, s2, settle_mge, settle_head, settle_stages, results,
                                   baseline, {"plugin": s2, "mge": settle_head}, False, log_path)
            with open(log_path, encoding="utf-8") as log:
                return code, log.read()

        # The null: a FAIL round names no move, and the ref stays.
        code, logged = settled([False])
        assert code == 1 and "moves" not in logged and resolve(settle_plugin, settle_ref) == s1, logged
        assert "%s: %s stays at %s: a stage did not pass" % (PLUGIN_NAME, settle_ref, s1[:10]) in logged, logged
        # A pass against another folder keeps the ref, with its line in the log.
        code, logged = settled([True], baseline=os.path.join(settle, "other"))
        assert code == 0 and resolve(settle_plugin, settle_ref) == s1, logged
        assert "%s: %s stays at %s: this pass is against %s, not the accepted folder" % (
            PLUGIN_NAME, settle_ref, s1[:10], os.path.join(settle, "other")) in logged, logged
        # The plant: a pass moves the ref, and the log names the old and the new value, the pair and the sibling.
        code, logged = settled([True])
        assert code == 0 and resolve(settle_plugin, settle_ref) == s2, logged
        assert "%s: %s moves from %s to %s: this round passed" % (PLUGIN_NAME, settle_ref, s1[:10], s2[:10]) in logged, logged
        assert "this round's pair: %s in the sibling %s: %s" % (pair_ref("plugin", s2), MGE_NAME, settle_head[:10]) in logged, logged
        assert "the sibling %s: refs/merge-step/feature/y stays absent" % MGE_NAME in logged, logged
        code, logged = settled([True])
        assert "is at %s already" % s2[:10] in logged, logged
        assert tips_for("mge", sibling_tree, "base", first)[1].startswith("no sibling"), "no sibling tree"
        # The A/B stages replay with the game slot held: their round takes both (D208).
        assert round_slot([("a", root, good), (INSTANCE_STAGE, root, good)]) == "both"

        # The exit code 5 of a stage is NOT RUN, with its reason, and the stage does not count as passed.
        not_run = [sys.executable, "-c", "print('replay base: exit code 0'); print('NOT RUN grass: the game slot is held by a run on Outlander-test: x'); raise SystemExit(%d)" % NOT_RUN_CODE]
        with open(os.path.join(root, "log6.txt"), "w", encoding="utf-8") as log:
            assert not run_stage("instance A/B grass", root, not_run, log)
        with open(os.path.join(root, "log6.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== NOT RUN instance A/B grass (" in text and "): the game slot is held by a run on Outlander-test: x\n" in text and "== FAIL" not in text and "== PASS" not in text, text
        assert not_run_reason("a\nb") == "the stage could not judge"

        # A FORMAT CHANGE (W527): the instance A/B's exit code 6 (the base refused the capture with 10 or 11, the build under
        # test played it). The exit code decides, never the text; the stage passes only with an evidence file, whose md5
        # goes into the close line.
        change = "NOT RUN grass: FORMAT CHANGE (the base refuses the capture with code 11, the extension format; the tip plays it)"
        evidence_file = os.path.join(root, "grass-evidence.txt")
        with open(evidence_file, "w", encoding="utf-8", newline="\n") as file:
            file.write("(a) OLD capture: equal. (b) NEW capture: two replays equal. (c) hash line: x\n")
        evidence_md5 = file_md5(evidence_file)

        def ab_round(code, printed=change, evidence=None, stage_name=INSTANCE_STAGE):
            EVIDENCE.clear()
            if evidence:
                EVIDENCE.update(evidence_of(["%s=%s" % (INSTANCE_STAGE, evidence)]))
            stage = [sys.executable, "-c", "print(%r); raise SystemExit(%d)" % (printed, code)]
            stages = [("a", root, good), (stage_name, root, stage)]
            with open(os.path.join(root, "log-ab-format.txt"), "w", encoding="utf-8") as log, contextlib.redirect_stdout(io.StringIO()):
                results = run_stages(stages, log)
            with open(os.path.join(root, "log-ab-format.txt"), encoding="utf-8") as log:
                text = log.read()
            closed = close_after(results, BASELINE, [("c" * 40, "Merge x (W1)")], lambda merges, stages: stages)
            EVIDENCE.clear()
            return results, text, closed

        try:
            # The plant: base 11, tip 0, no file: the round fails, and the line says how to give the file.
            results, text, closed = ab_round(FORMAT_CHANGE_EXIT)
            assert results == [True, False] and closed is None, (results, text)
            assert "== NOT RUN instance A/B grass (" in text and 'the round fails without an evidence file: --evidence "instance A/B grass=FILE"' in text, text
            # With the file: the stage passes, and the close line has the verdict and the md5.
            results, text, closed = ab_round(FORMAT_CHANGE_EXIT, evidence=evidence_file)
            assert results == [True, True] and "== PASS instance A/B grass (" in text and "md5 %s" % evidence_md5 in text, text
            assert closed.startswith("PASS 2 of 2: a, instance A/B grass; instance A/B grass: FORMAT CHANGE (") and closed.endswith("evidence %s md5 %s" % (os.path.abspath(evidence_file), evidence_md5)), closed
            # The nulls: a tip that refuses the capture too is a FAIL of the stage (instance_ab exits 1); a NOT RUN (5) of the
            # stage fails with the file, even when its text says FORMAT CHANGE; a stage of another name with code 6 FAILs.
            assert ab_round(1, "FAIL grass: the base refuses the capture with code 11, and the replay of the build under test exited with code 11", evidence=evidence_file)[0] == [True, False]
            results, text, closed = ab_round(NOT_RUN_CODE, evidence=evidence_file)
            assert results == [True, False] and closed is None and "== PASS instance A/B grass" not in text, text
            assert ab_round(FORMAT_CHANGE_EXIT, stage_name="a check")[0] == [True, False]
            # A full pass with a file names no FORMAT CHANGE in the close line.
            assert ab_round(0, "PASS grass: x", evidence=evidence_file)[2] == "PASS 2 of 2: a, instance A/B grass"
        finally:
            EVIDENCE.clear()
        # The evidence argument: only the instance A/B takes a file, once, and the file must exist.
        assert evidence_of(None) == {} and evidence_of(["%s=%s" % (INSTANCE_STAGE, evidence_file)]) == {INSTANCE_STAGE: (os.path.abspath(evidence_file), evidence_md5)}
        assert "only the stage" in evidence_of(["replay gate=%s" % evidence_file])
        assert "no such file" in evidence_of(["%s=%s" % (INSTANCE_STAGE, os.path.join(root, "none.txt"))])
        assert "give STAGE=FILE" in evidence_of([INSTANCE_STAGE]) and "a second file" in evidence_of(["%s=%s" % (INSTANCE_STAGE, evidence_file)] * 2)
        # The base is never the tip.
        assert base_problem("c" * 40, "c" * 40, True, "b" * 40, True).startswith("the base is the commit")

        # The A/B stage does not run when the round's set or the base set was not made: a set that the maker withheld (it
        # counts as passed) by its own reason, a failed set by the stop at the first failure (W567 part B).
        with open(os.path.join(root, "log7b.txt"), "w", encoding="utf-8") as log:
            assert run_stages([(SET_STAGE, root, bad), gate_stage, (BASE_STAGE, root, never), (INSTANCE_STAGE, root, never)], log) == [False, False, False, False]
            assert run_stages([(SET_STAGE, root, maker_missing), (INSTANCE_STAGE, root, never)], log) == [True, False]
        with open(os.path.join(root, "log7b.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== NOT RUN instance A/B grass: %s %s" % (SET_STAGE, stop) in text and "== NOT RUN instance A/B grass: the round's set was not made" in text, text
        with open(os.path.join(root, "log8b.txt"), "w", encoding="utf-8") as log:
            assert run_stages([(SET_STAGE, root, good), gate_stage, (BASE_STAGE, root, bad), (INSTANCE_STAGE, root, never)], log) == [True, True, False, False]
        with open(os.path.join(root, "log8b.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== NOT RUN instance A/B grass: %s %s" % (BASE_STAGE, stop) in text, text

        # A pass against a named folder does not move the ref.
        assert moves_the_ref(BASELINE) and not moves_the_ref(os.path.join(root, "copy"))

        # A baseline with no capture refuses a round with the replay gate before its take and builds (Rob's run
        # 8b51a23d59 failed the gate at 0 s after its builds); a round with no gate needs no baseline.
        empty, filled = os.path.join(root, "empty-baseline"), os.path.join(root, "filled-baseline")
        os.makedirs(empty)
        os.makedirs(filled)
        open(os.path.join(filled, "seyda.nircap"), "wb").close()
        gate_round = stages_for("plugin", plugin, ["host/src/material.rs"], empty)
        problem = baseline_problem(gate_round, empty)
        assert problem and empty in problem and "no capture" in problem, problem
        assert baseline_problem(gate_round, os.path.join(root, "no-such-folder")), "a missing folder"
        assert baseline_problem(stages_for("plugin", plugin, ["host/src/material.rs"], filled), filled) is None
        assert baseline_problem(stages_for("plugin", plugin, ["src/lib.rs"], empty), empty) is None, "no gate, no baseline"

        # A refusal for lack of room ends the wait, and a place in the queue goes on.
        class Slots:
            DONE, REFUSED, QUEUED = 0, 1, 3
            STATE_DIR = os.path.join(root, "slots")
            StateBusy = OSError
            replies = []

            @classmethod
            def take(cls, *_, **__):
                reply = cls.replies.pop(0)
                code, text = reply if isinstance(reply, tuple) else (reply, "a reply\nmore")
                return types.SimpleNamespace(code=code, text=text)

            @staticmethod
            def leave(*_):
                pass

        Slots.replies = [Slots.REFUSED]
        assert take_build(Slots, "x", False) is False
        # A take prints every line of the reply (the free space on C follows); a refusal or a wait prints the first.
        Slots.replies = [Slots.DONE]
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            assert take_build(Slots, "x", False) is True
        assert shown.getvalue() == "a reply\nmore\n", shown.getvalue()
        Slots.replies = [Slots.REFUSED]
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            assert take_build(Slots, "x", False) is False
        assert shown.getvalue() == "NOT RUN: the take of the build slot was refused: a reply\n", shown.getvalue()
        Slots.replies = [Slots.QUEUED]
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            assert take_build(Slots, "x", True) is False and Slots.replies == []
        assert shown.getvalue() == "NOT RUN: --no-wait, and the build slot is not free: a reply\n", shown.getvalue()

        # Another run of this session holds the slot (2026-11-08 23:22:44, the Coordinator): this run waits for its
        # release, with one line, and then takes the slot. With --no-wait it ends with NOT RUN.
        own = (Slots.REFUSED, 'build: refused: this session holds it as "merge_step: other" (since t); a take with another label is a second run.\nRun...')
        Slots.replies = [own, own, own, Slots.DONE]
        sleeps = []
        real_sleep, time.sleep = time.sleep, sleeps.append
        try:
            shown = io.StringIO()
            with contextlib.redirect_stdout(shown):
                assert take_build(Slots, "x", False) is True and Slots.replies == []
        finally:
            time.sleep = real_sleep
        lines = shown.getvalue().splitlines()
        assert len(sleeps) == 3 and lines[0].startswith("waiting: another run of this session holds the build slot") and lines[1:] == ["a reply", "more"], lines
        Slots.replies = [own]
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            assert take_build(Slots, "x", True) is False
        assert shown.getvalue().startswith("NOT RUN: --no-wait, and the build slot is not free: build: refused: this session holds it"), shown.getvalue()
        # The plainer form of the refusal waits alike.
        plainer = (Slots.REFUSED, 'build: refused: your session already holds the build slot as "merge_step: other" (since t); work in this session runs under that hold')
        Slots.replies = [plainer, Slots.DONE]
        real_sleep, time.sleep = time.sleep, sleeps.append
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                assert take_build(Slots, "x", False) is True and Slots.replies == []
        finally:
            time.sleep = real_sleep

        # A round with the replay gate takes the game and the whole build side in one take (D208); one without takes a place.
        gate = [("plugin tests", root, good), ("replay gate", root, good)]
        assert round_slot(gate) == "both" and round_slot(gate[:1]) == "build"
        asked = []
        Slots.take = classmethod(lambda cls, *args, **options: asked.append((args, options)) or types.SimpleNamespace(code=cls.DONE, text="a reply"))
        assert take_build(Slots, "x", False) is True and asked == [(("build", "x"), {"minutes": SLOT_MINUTES})], asked
        # The lease of a take of both is the game slot's, longer than the build slot's.
        asked.clear()
        assert take_slot(Slots, "both", "x", False, minutes=GAME_SLOT_MINUTES) is True and asked == [(("both", "x"), {"minutes": GAME_SLOT_MINUTES})], asked
        assert GAME_SLOT_MINUTES > SLOT_MINUTES
        # A slot.py with --another (W361) gets it on each take, beside the other options; one without it gets none (above).
        asked.clear()
        Slots.ANOTHER = "another"
        try:
            assert take_slot(Slots, "build", "x", False, {"whole": True}) is True
        finally:
            del Slots.ANOTHER
        assert asked == [(("build", "x"), {"minutes": SLOT_MINUTES, "whole": True, "another": True})], asked

        # With slot.py's LoopWaiter, a leave of the place ends the wait: the next ask returns WAIT_ENDED, and the run is
        # NOT RUN with no take (2026-11-09 07:51: a loop of plain takes took the whole side after its leave). The plain
        # take of this fake gives the slot, so a loop that does not ask through the waiter takes it.
        class Waiting(Slots):
            WAIT_ENDED = 5
            asks = []

            class LoopWaiter:
                def __init__(self, slot):
                    Waiting.asks.append(("enter", slot))

                def __enter__(self):
                    return self

                def __exit__(self, *_):
                    Waiting.asks.append(("exit",))

                def take(self, who, **_):
                    Waiting.asks.append(("ask", who))
                    return types.SimpleNamespace(code=Waiting.replies.pop(0), text="build: this wait ended without the slot: the place was left or dropped\nmore")

        Waiting.replies = [Slots.QUEUED, Waiting.WAIT_ENDED]
        real_sleep, time.sleep = time.sleep, sleeps.append
        try:
            shown = io.StringIO()
            with contextlib.redirect_stdout(shown):
                assert take_build(Waiting, "x", False) is False and Waiting.replies == []
        finally:
            time.sleep = real_sleep
        assert Waiting.asks == [("enter", "build"), ("ask", "x"), ("ask", "x"), ("exit",)], Waiting.asks
        assert shown.getvalue().splitlines()[-1] == "NOT RUN: this run left the queue of the build slot: build: this wait ended without the slot: the place was left or dropped", shown.getvalue()

        # A stage of tests fails when no test ran: cargo passes a filter that matches nothing, and a lost #[ignore] leaves 0.
        one = "running 1 test\n.\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 340 filtered out; finished in 0.1s\n"
        none = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 341 filtered out; finished in 0.0s\n"
        failed = "test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n"
        assert tests_ran(one) == 1 and tests_ran(none) == 0 and tests_ran(failed) == 3 and tests_ran(one + none + failed) == 4 and tests_ran("") == 0

        def prints(text, code=0):
            return [sys.executable, "-c", "import sys; sys.stdout.write(%r); sys.exit(%d)" % (text, code)]

        for output, code, wanted in ((one, 0, 0), (none, 0, 1), ("", 0, 1), (failed, 101, 101), (none, 101, 101)):
            shown = io.StringIO()
            with contextlib.redirect_stdout(shown):
                assert require_tests(prints(output, code)) == wanted, (output, code)
        assert "0 tests ran" in shown.getvalue()
        # Through the stage: the same exit codes decide the line.
        wrapper = [sys.executable, "-I", os.path.abspath(__file__), REQUIRE_TESTS]
        with open(os.path.join(root, "log9.txt"), "w", encoding="utf-8") as log:
            assert run_stage("with a test", root, wrapper + prints(one), log)
            assert not run_stage("with none", root, wrapper + prints(none), log)
        with open(os.path.join(root, "log9.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== PASS with a test (" in text and "== FAIL with none (" in text and "no test ran" in text, text

        # W523 (the audit's F-H-2): a plugin round that changes anything under host/src/ runs every test of the helper's
        # package in the render-link tree of the pair; a path outside, and a render-link round, do not. The stage's name
        # has the commit of that tree; its command is the wrapper, so 0 tests fail it.
        for changed in ("host/src/shader.wgsl", "host/src/stage_shaders.rs", "host/src/depth.rs", "host/src/player/draws.rs"):
            planned = [stage for stage in stages_for("plugin", plugin, [changed]) if stage[0].startswith(HELPER_TEXT_STAGE)]
            assert len(planned) == 1 and planned[0][0] == HELPER_TEXT_STAGE and planned[0][1] == mge, (changed, planned)
            assert planned[0][2] == wrapper[:2] + [os.path.join(plugin, "tools", "merge_step.py"), REQUIRE_TESTS, "cargo", "test", "--quiet", "-p", "mgeHost64"], planned[0][2]
        for kind, repo, changed in (("plugin", plugin, "src/lib.rs"), ("plugin", plugin, "host/Cargo.toml"), ("plugin", plugin, "host/src_old/x.rs"), ("plugin", plugin, "tools/x.py"),
                                    ("mge", mge, "mgeHost64/src/renderer.rs")):
            assert not any(name.startswith(HELPER_TEXT_STAGE) for name, _, _ in stages_for(kind, repo, [changed])), (kind, changed)
        named = helper_text_stage(plugin, mge, under_test("plugin", "a" * 40, "b" * 40))
        assert named[0] == HELPER_TEXT_STAGE + " (render-link bbbbbbbbbb)" and named[1:] == planned[0][1:], named
        in_round = [name for name, _, _ in stages_for("plugin", plugin, ["host/src/material.rs"], commits=under_test("plugin", "a" * 40, "b" * 40))]
        assert in_round.index(named[0]) == in_round.index("docs") + 1 < in_round.index(SET_STAGE), in_round
        # The stage's wrapper (this file in place of the planted tree's) with a planted output: a test that ran passes,
        # 0 tests fail.
        assert named[2][3] == REQUIRE_TESTS and named[2][:2] == wrapper[:2]
        with open(os.path.join(root, "log9b.txt"), "w", encoding="utf-8") as log:
            assert run_stage(named[0], root, wrapper + prints(one), log)
            assert not run_stage(named[0], root, wrapper + prints(none), log)
        with open(os.path.join(root, "log9b.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== PASS %s (" % named[0] in text and "== FAIL %s (" % named[0] in text and "no test ran" in text, text

        # The slots of a round (the owner's D161, D208): a round that needs the game takes the game and the whole build side in
        # one take, holds them for all its stages, and releases both on every exit path. This uses the real slot.py on a
        # scratch state folder, so that no slot of the day is touched.
        slots = scratch_slots(os.path.join(root, "slot-state"))
        state = slots.STATE_DIR
        assert state.startswith(root)
        # The rounds of this part test the holds of a check that needs the game slot; their stages have test names, which
        # GAME_STAGES does not hold. The part of D183 below uses the real rule.
        real_needs_game = globals()["needs_game"]
        globals()["needs_game"] = lambda stages: True
        own_session = os.environ["CLAUDE_CODE_SESSION_ID"]
        other_session = "merge-step-self-test-other"

        def as_session(name):
            os.environ["CLAUDE_CODE_SESSION_ID"] = name

        def holders():
            return sorted(name for name in os.listdir(state) if name.endswith(".holder"))

        all_held = [sys.executable, "-c", "import os, sys; sys.exit(0 if all(os.path.exists(p) for p in %r) else 7)" % (
            [slots.holder_path(unit) for unit in ("game",) + tuple(slots.units_of("build"))],)]
        round_log = os.path.join(root, "round.log")
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            # The take: the game and both build places are held while the stage runs, and the stage checks it.
            assert run_round(slots, [("all held", root, all_held)], "merge_step: x aaaaaaaaaa", round_log, True)[0] == [True]
            assert holders() == [], "a pass releases both"
            # A failing stage.
            assert run_round(slots, [("all held", root, all_held), ("bad", root, bad)], "merge_step: x bbbbbbbbbb", round_log, True)[0] == [True, False]
            assert holders() == [], "a failing stage releases both"
            # An exception in a stage (no such folder), and a Ctrl-C.
            try:
                run_round(slots, [("lost", os.path.join(root, "no-such-folder"), good)], "merge_step: x cccccccccc", round_log, True)
                raise AssertionError("the exception did not reach the caller")
            except OSError:
                pass
            assert holders() == [], "an exception releases both"
            real_run_stages = globals()["run_stages"]

            def interrupted(*_, **__):
                raise KeyboardInterrupt

            globals()["run_stages"] = interrupted
            try:
                run_round(slots, [("a", root, good)], "merge_step: x dddddddddd", round_log, True)
                raise AssertionError("the interrupt did not reach the caller")
            except KeyboardInterrupt:
                pass
            finally:
                globals()["run_stages"] = real_run_stages
            assert holders() == [], "a Ctrl-C releases both"
            # Another session holds the game slot, or a place of the build side, and the run does not wait (--no-wait): it
            # takes nothing at all, not the free slot either.
            for part in ("game", "build"):
                as_session(other_session)
                assert slots.take(part, "other: " + part, minutes=5).code == slots.DONE
                as_session(own_session)
                assert run_round(slots, [("a", root, good)], "merge_step: x eeeeeeeeee", round_log, True) is None
                assert holders() == [part + ".holder"], holders()
                as_session(other_session)
                assert slots.release(part, "other: " + part).code == slots.DONE
                as_session(own_session)
        assert "NOT RUN: --no-wait, and the both slot is not free" in shown.getvalue(), shown.getvalue()

        # D208: a round that waits holds nothing meanwhile, so that no other session can wait for it (the deadlock of 14:37 on
        # 2026-11-10). Another session holds a place; the round waits in a thread with a deadline; it has a place in both
        # queues and no hold; after the release it takes both and runs.
        as_session(other_session)
        assert slots.take("build", "other: build", minutes=5).code == slots.DONE
        as_session(own_session)
        real_wait_seconds = globals()["WAIT_SECONDS"]
        globals()["WAIT_SECONDS"] = 0.05
        outcome = {}

        def waiting_round():
            try:
                outcome["round"] = run_round(slots, [("all held", root, all_held)], "merge_step: x ffffffffff", round_log, False)
            except BaseException as error:  # noqa: BLE001 - the case reports it
                outcome["error"] = error

        def own_places():
            return [part for part in SLOTS_OF_BOTH if any(ticket.session == own_session for ticket in slots.read_tickets(part))]

        output = io.StringIO()
        try:
            with contextlib.redirect_stdout(output):
                thread = threading.Thread(target=waiting_round, daemon=True)
                thread.start()
                deadline = time.monotonic() + 30
                while own_places() != list(SLOTS_OF_BOTH) and time.monotonic() < deadline:
                    time.sleep(0.05)
                places_while_waiting, holds_while_waiting = own_places(), holders()
                # The other session releases in a process of its own: a change of the session variable here would also
                # change the session of the waiting round's next ask, which then ends its own wait.
                released = subprocess.run([sys.executable, "-B", os.path.join(HARNESS, "slot.py"), "release", "build", "other: build"],
                                          env=dict(os.environ, CLAUDE_CODE_SESSION_ID=other_session), capture_output=True, text=True)
                assert released.returncode == slots.DONE, released.stdout + released.stderr
                thread.join(30)
        finally:
            globals()["WAIT_SECONDS"] = real_wait_seconds
            as_session(own_session)
        assert places_while_waiting == list(SLOTS_OF_BOTH), "the waiting round has no place in both queues: %s" % places_while_waiting
        assert holds_while_waiting == ["build.holder"], "the waiting round holds a slot: %s" % holds_while_waiting
        assert not thread.is_alive() and "error" not in outcome, outcome
        assert outcome["round"] is not None and outcome["round"][0] == [True], (outcome, output.getvalue())
        assert holders() == [], holders()

        # A hold of the round lapses (a stall: the notes of the work stop): before the next stage the run takes it again when it
        # is free.
        def lapse_now():
            slots.lapse_idle_holds(time.time() + 20 * 60)

        real_note_active = slots.note_active
        build_notes = []

        def note_then_lapse_before_the_second_stage(slot, now):
            real_note_active(slot, now)
            if slot == "build":
                build_notes.append(now)
                if len(build_notes) == 2:
                    lapse_now()

        slots.note_active = note_then_lapse_before_the_second_stage
        shown = io.StringIO()
        try:
            with contextlib.redirect_stdout(shown):
                lapsed_round = run_round(slots, [("first", root, all_held), ("second", root, all_held), ("third", root, all_held)],
                                         "merge_step: x iiiiiiiiii", round_log, True)
        finally:
            slots.note_active = real_note_active
        assert lapsed_round is not None and lapsed_round[0] == [True, True, True], (lapsed_round, shown.getvalue())
        assert "re-took the build side" in shown.getvalue() and "re-took the game slot" in shown.getvalue(), shown.getvalue()
        with open(round_log, encoding="utf-8") as log:
            kept = log.read()
        assert "== re-took the build side at " in kept and "== re-took the game slot at " in kept, "the kept log names each take again: " + kept
        assert holders() == [], holders()

        # A lapsed hold that another session took meanwhile is not waited for: the run holds the game, and a wait for the build
        # side would be a hold and wait (D208). The stage is NOT RUN; a hold that is intact is left alone.
        what = "merge_step: x jjjjjjjjjj"
        assert slots.take("both", what, minutes=5).code == slots.DONE
        intact = io.StringIO()
        assert keep_holds(slots, what, "both", intact) is True and intact.getvalue() == "", intact.getvalue()
        assert slots.release("build", what).code == slots.DONE
        as_session(other_session)
        assert slots.take("build", "other: build", minutes=5).code == slots.DONE
        as_session(own_session)
        refused = io.StringIO()
        seen_before = [slots.last_active(unit) for unit in slots.units_of("build")]
        with contextlib.redirect_stdout(io.StringIO()) as said:
            assert keep_holds(slots, what, "both", refused) is False, "the run waited for the build side with the game held"
        assert [slots.last_active(unit) for unit in slots.units_of("build")] == seen_before, "the run notes build work for a hold of another session"
        assert "taking it again if it is free" in refused.getvalue() and "NOT RUN" in said.getvalue(), (refused.getvalue(), said.getvalue())
        assert places_held(slots, "build", "merge_step: x some-other-run") == 0, "a hold of another label is no hold of the run"
        assert not any(ticket.session == own_session for ticket in slots.read_tickets("build")), "the run waits in the build queue"
        as_session(other_session)
        assert slots.release("build", "other: build").code == slots.DONE
        as_session(own_session)
        # A hold of the game alone (the last stages of a play set, W405): kept while it stands, NOT RUN when another
        # session took it, as a wait would give the game away between two stages.
        alone = io.StringIO()
        assert keep_holds(slots, what, "game", alone) is True and alone.getvalue() == "", alone.getvalue()
        assert slots.release("game", what).code == slots.DONE
        as_session(other_session)
        assert slots.take("game", "other: a game", minutes=5).code == slots.DONE
        as_session(own_session)
        with contextlib.redirect_stdout(io.StringIO()) as said:
            assert keep_holds(slots, what, "game", io.StringIO()) is False, "the run waited for the game"
        assert "NOT RUN" in said.getvalue(), said.getvalue()
        assert not any(ticket.session == own_session for ticket in slots.read_tickets("game")), "the run waits in the game queue"
        as_session(other_session)
        assert slots.release("game", "other: a game").code == slots.DONE
        as_session(own_session)
        assert holders() == [], holders()
        with open(os.path.join(root, "stopped.log"), "w+", encoding="utf-8") as stopped:
            assert run_stages([("a", root, good), ("b", root, good)], stopped, lambda: False) == [False, False]
            stopped.seek(0)
            assert "== NOT RUN a: the slot hold of this run ended and was not taken again" in stopped.read()
        globals()["needs_game"] = real_needs_game

        # D183: the game slot only for a stage that needs it. The stage lists of the tool first.
        assert not needs_game(stages_for("plugin", plugin, ["tools/x.py", "host/src_old/x.rs"])), "the checks of a tools change took the game"
        assert needs_game(stages_for("plugin", plugin, ["host/src/material.rs"])), "the replay gate did not take the game"
        assert not needs_game(stages_for("mge", mge, ["xtask/src/main.rs"]))
        assert needs_game([(INSTANCE_STAGE, root, good)]) and not needs_game([("checks", root, good)])
        # The stage passes only when the round holds the build side, and has no hold and no waiter of the game slot.
        build_only = [sys.executable, "-c", "import os, sys; sys.exit(0 if os.path.exists(%r) and not os.path.exists(%r) and not "
                      "any(name.startswith('game.waiter.') for name in os.listdir(%r)) else 7)"
                      % (slots.holder_path("build"), slots.holder_path("game"), slots.STATE_DIR)]
        real_note_active = slots.note_active
        noted_slots = []
        slots.note_active = lambda slot, now: (noted_slots.append(slot), real_note_active(slot, now))[1]
        with contextlib.redirect_stdout(io.StringIO()) as quiet:
            # A round with no game stage holds the build side only, notes no work of the game, and says so.
            try:
                assert run_round(slots, [("checks", root, build_only)], "merge_step: x ffffffffff", round_log, True)[0] == [True], "the round held the game"
            finally:
                slots.note_active = real_note_active
            assert "game" not in noted_slots, "a round with no game stage noted the work of the game: %s" % noted_slots
            assert holders() == [], holders()
            assert "need no game slot" in quiet.getvalue(), quiet.getvalue()
            # Another session's game does not stop it, and keeps its hold.
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test-other"
            assert slots.take("game", "other: a game", minutes=5).code == slots.DONE
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test"
            assert run_round(slots, [("checks", root, good)], "merge_step: x 1111111111", round_log, True)[0] == [True], "a game stopped a check with no game stage"
            assert holders() == ["game.holder"], holders()
            # A round with a stage that needs the game still waits for it: with --no-wait it stops.
            assert run_round(slots, [(INSTANCE_STAGE, root, good)], "merge_step: x 2222222222", round_log, True) is None, "a game stage ran beside a game"
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test-other"
            assert slots.release("game", "other: a game").code == slots.DONE
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test"
            # With the game free, it holds the game and the whole build side while the stage runs, and releases them.
            assert run_round(slots, [(INSTANCE_STAGE, root, all_held)], "merge_step: x 3333333333", round_log, True)[0] == [True]
            assert holders() == [], holders()

        # The notes of the game slot's work: one at once, then one each period, and none after the round.
        noted = []
        counting = types.SimpleNamespace(note_active=lambda slot, now: noted.append(slot))
        with game_hold_noted(counting, 0.01):
            time.sleep(0.2)
        after = len(noted)
        time.sleep(0.1)
        assert after >= 3 and set(noted) == {"game"} and len(noted) == after, noted
        # With the label of a take, the notes count the build work of that take's places too (W405).
        noted.clear()
        counting.session_id = lambda: "s"
        counting.owner_of = lambda session, who: "%s/%s" % (session, who)
        counting.note_holder_build = lambda owner, now: noted.append("build " + owner)
        with game_hold_noted(counting, 0.01, build_who="make_play_set: x"):
            time.sleep(0.1)
        assert noted[:2] == ["game", "build s/make_play_set: x"] and noted.count("game") == noted.count("build s/make_play_set: x"), noted

        # The log keeps its name, and its folder is made.
        logs = os.path.join(root, "new", "logs")
        first_log = log_path_of("0123456789abcdef", logs)
        assert first_log == os.path.join(logs, "merge-step-0123456789.log") and os.path.isdir(logs)
        # A rerun for the same commit never writes into the log of an earlier round.
        with open(first_log, "w", encoding="utf-8") as log:
            log.write("the refused round\n")
        second_log = log_path_of("0123456789abcdef", logs)
        third_log = log_path_of("0123456789abcdef", logs)
        assert (second_log, third_log) == (os.path.join(logs, "merge-step-0123456789-2.log"), os.path.join(logs, "merge-step-0123456789-3.log"))
        with open(second_log, "w", encoding="utf-8") as log:
            log.write("the rerun\n")
        with open(first_log, encoding="utf-8") as log:
            assert log.read() == "the refused round\n", "the rerun wrote into the log of the first round"
        assert log_path_of("fedcba9876543210", logs) == os.path.join(logs, "merge-step-fedcba9876.log"), "another commit starts at the plain name"
        assert "target" not in LOG_FOLDER.lower()

        # The closing lines of the items (D180): only a subject that ends with "(W<n>)" or "(W<n> <part>)" names an item.
        assert items_of("Merge: a check that MWSE's vtable fills name their own slot (W93)") == ["W93"]
        # W357: the item of a part keeps its part, so that a row with open parts does not close; a subject without a part
        # closes the whole row.
        assert items_of("Merge: the second part (W234 a)") == ["W234 a"]
        assert items_of("Merge: the crate half (W229 crate half) ") == ["W229 crate half"]
        assert items_of("Merge: d3d8 finds the plugin's exports by name (W173 line 3)") == ["W173 line 3"]
        assert items_of("Merge: the pass report (W341 step A)") == ["W341 step A"]
        assert items_of("Merge: the pass report, all steps (W341)") == ["W341"]
        assert items_of("Merge: a part with two spaces (W341  step  A)") == ["W341 step A"]
        # W365: a tail of decisions after the item is dropped; any other comma stays in the part.
        assert items_of("Merge: the cleaners keep the game free for their run (W363, D325)") == ["W363"]
        assert items_of("Merge: GPU objects are counted, and a steady frame makes none (W340, D288/D289/D310)") == ["W340"]
        assert items_of("Merge: the state once in frame() (W341 step B, D300)") == ["W341 step B"]
        assert items_of("Merge: the view model has no part REFLECTION (W341 step B, commit 1)") == ["W341 step B, commit 1"]
        assert items_of("Merge: a part and two decisions (W341 step B, D300/D301)") == ["W341 step B"]
        for no_item in ("Merge: W96 in the middle of the subject", "Merge: W96 in the middle (D161)", "Merge: dropped W220",
                        "Merge: log pipe follow-ups W219 and W221 (W220 dropped) and more", "Merge: lower case (w96)",
                        "Merge: log pipe follow-ups W219 and W221 (W220 dropped)", "Merge: a nested part (W234 (a))",
                        "Merge: a dropped item (W96 dropped)", "Merge: the item (W96) is not at the end",
                        "Merge: no parentheses W96", "Merge: two items (W222, W226)", "Merge: the plan (D161)",
                        "Merge: say which views draw the table", "Merge: a letter on the number (W234a)", "Merge: empty ()",
                        "Merge: a dropped item with a decision (W96 dropped, D5)", "Merge: a tail only (D325)",
                        "Merge: a tail with lower case (W363, d325)", "Merge: a tail without a number (W363, D)",
                        "Merge: a tail with no space (W363,D325)"):
            assert items_of(no_item) == [], no_item
        # W528: the subject of bdc4145 named no item, as the tail held a row of another item. A tail of references closes
        # the item: rows of other items, several decisions, a risk, an E item.
        w527 = "Merge: a merge-step stage cannot pass on nothing (W527, W524 row 2)"
        assert items_of(w527) == ["W527"], items_of(w527)
        assert items_of("Merge: a run takes the run folder's plugin copy (W520, D488, D489)") == ["W520"]
        assert items_of("Merge: free a texture level's CPU copy at its first unlock (W18, risk 42)") == ["W18"]
        assert items_of("Merge: a fragment shader for each stage state of the draws (W293, E2)") == ["W293"]
        assert items_of("Merge: a part with a row tail (W341 step B, W524 row 2, D300)") == ["W341 step B"]
        assert unread_item_lines([("b" * 40, w527)]) == [], "an item that is read gives no warning"
        # The null: a subject that names no item closes nothing, and one with a W number says so.
        for no_item in ("Merge: two items (W222, W226)", "Merge: the exit paths do not wait for stdout (W229, crate half)",
                        "Merge: a letter on the number (W234a)", "Merge: D first (D208, W294)", "Merge: a bare row tail (W389, W289)",
                        "Merge: a dropped row in the tail (W1, W2 dropped)"):
            assert items_of(no_item) == [], no_item
            warned = unread_item_lines([("a" * 40, no_item)])
            assert len(warned) == 1 and warned[0].startswith("WARNING: %s closes nothing" % ("a" * 10)) and no_item in warned[0], warned
        assert unread_item_lines([("a" * 40, "Merge: the plan (D161)"), ("a" * 40, "Merge: say which views draw the table")]) == []

        closes = os.path.join(root, "closes", "closes.tsv")
        os.makedirs(os.path.dirname(closes))
        when = "2026-11-09T20:15:00+04:00"
        round_log = "E:\\nirender-keep\\merge-step\\merge-step-862873b012.log"

        def close_batch(results, merges, baseline=BASELINE):
            words = [("s%d" % number, PASS_WORD if passed else FAIL_WORD, "") for number, passed in enumerate(results)]
            return close_after(results, baseline, merges, lambda merges, stages: write_closes(closes, when, PLUGIN_NAME, merges, stages, round_log, wait=0),
                               words)

        def close(results, subject, commit="862873b0123456789", baseline=BASELINE):
            return close_batch(results, [(commit, subject)], baseline)

        def closes_bytes():
            with open(closes, "rb") as written:
                return written.read()

        # W528, the plant: the subject of bdc4145 writes its close.
        w527_line = "\t".join((when, "W527", PLUGIN_NAME, "bdc4145a35", "PASS 6 of 6: s0, s1, s2, s3, s4, s5", round_log, w527))
        assert close([True] * 6, w527, commit="bdc4145a35d") == [w527_line]
        os.remove(closes)
        subject = "Merge: a check that MWSE's vtable fills name their own slot (W93)"
        # A failed stage writes nothing and makes no file; so do a pass against another folder and a subject with no item.
        assert close([True, False, True, True, True, True], subject) is None and not os.path.exists(closes), "a fail wrote a close"
        assert close([], subject) is None and not os.path.exists(closes), "a round with no stage wrote a close"
        assert close([True] * 6, subject, baseline=os.path.join(root, "other")) is None and not os.path.exists(closes)
        assert close([True] * 6, "Merge: say which views draw the table") == [] and not os.path.exists(closes), "a subject with no item wrote"
        # A pass writes the header and one line, CRLF.
        line = "\t".join((when, "W93", PLUGIN_NAME, "862873b012", "PASS 6 of 6: s0, s1, s2, s3, s4, s5", round_log, subject))
        assert close([True] * 6, subject) == [line]
        assert closes_bytes() == (CLOSES_HEADER + "\r\n" + line + "\r\n").encode("ascii"), closes_bytes()
        # A rerun of the commit writes nothing; a later commit of the item writes its line, also in the part form; an item
        # in the middle of a subject writes nothing.
        assert close([True] * 6, subject) == [], "a rerun wrote a second line"
        assert len(close([True] * 5, "Merge: part b (W93)", commit="abcdef0123456")) == 1
        assert [written.split("\t")[1] for written in close([True] * 4, "Merge: the second part (W234 b)", commit="1111111111aa")] == ["W234 b"]
        assert close([True] * 6, "Merge: W96 in the middle of the subject", commit="3333333333") == []
        data = closes_bytes()
        assert data.count(b"\r\n") == 4 and data.count(CLOSES_HEADER.encode("ascii")) == 1 and b"\n" not in data.replace(b"\r\n", b""), data

        # A reader that holds the file: the append is tried again, then given up with None, and nothing else happens.
        tries = []

        def held(*args, **kwargs):
            tries.append(args[0])
            raise PermissionError("held by a reader")

        globals()["open"] = held
        try:
            given_up = write_closes(closes, when, PLUGIN_NAME, [("2222222222", "Merge: x (W300)")], "6 of 6", round_log, tries=3, wait=0)
        finally:
            del globals()["open"]
        assert given_up is None and len(tries) == 3, tries
        assert closes_bytes() == data, "a given-up append changed the file"

        # D183: a batch of merges with one check closes the item of each merge on the first-parent path, and the checked
        # commit's own; a commit of a merged branch closes nothing.
        entries = [("a1a1a1a1a1aa", "0000", "Fix a thing on the branch (W90)"), ("m1m1m1m1m1mm", "a0a0 b1b1", "Merge: one (W96)"),
                   ("m2m2m2m2m2mm", "m1m1 c1c1", "Merge: two (W97 a)"), ("m3m3m3m3m3mm", "m2m2 d1d1", "Merge: no item")]
        assert closing_commits(entries, "m3m3m3m3m3mm") == [(sha, subject) for sha, _, subject in entries[1:]], "the merges of the path"
        assert closing_commits([("t1t1t1t1t1tt", "0000", "Merge: a fast-forward (W98)")], "t1t1t1t1t1tt") == [("t1t1t1t1t1tt", "Merge: a fast-forward (W98)")]
        batch = closing_commits(entries, "m3m3m3m3m3mm")
        assert close_batch([True, False], batch) is None, "a failed batch wrote a close"
        assert [line.split("\t")[1:4] for line in close_batch([True] * 6, batch)] == [["W96", PLUGIN_NAME, "m1m1m1m1m1"], ["W97 a", PLUGIN_NAME, "m2m2m2m2m2"]], "a batch"
        assert close_batch([True] * 6, batch) == [], "a rerun of the batch wrote again"
        assert close_batch([True] * 6, [("m4m4m4m4m4mm", "Merge: three (W99)"), ("m4m4m4m4m4mm", "Merge: three (W99)")]) != [], "one line, not two"
        assert closes_bytes().count(b"\tW99\t") == 1, "one commit and item twice in a batch wrote two lines"

        # The first-parent path of a real history: two merges of two branches, each branch with an item in its own subject,
        # a commit on main between them, and a merge of main into the second branch (that merge is not on the path).
        history = os.path.join(root, "history")
        os.makedirs(history)

        def in_history(*args):
            return subprocess.run(["git", "-C", history, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", *args],
                                  capture_output=True, text=True, check=True, env=git_env()).stdout.strip()

        in_history("init", "-q", "-b", "main")
        in_history("commit", "-q", "--allow-empty", "-m", "the base")
        history_base = in_history("rev-parse", "HEAD")
        in_history("switch", "-q", "-c", "one")
        in_history("commit", "-q", "--allow-empty", "-m", "Fix on the branch (W12)")
        in_history("switch", "-q", "main")
        in_history("merge", "-q", "--no-ff", "-m", "Merge: one (W96)", "one")
        in_history("switch", "-q", "-c", "two")
        in_history("commit", "-q", "--allow-empty", "-m", "Fix on the branch (W13)")
        in_history("switch", "-q", "main")
        in_history("commit", "-q", "--allow-empty", "-m", "A fix on main (W14)")
        in_history("switch", "-q", "two")
        in_history("merge", "-q", "--no-ff", "-m", "Merge: main into two (W15)", "main")
        in_history("switch", "-q", "main")
        in_history("merge", "-q", "--no-ff", "-m", "Merge: two (W97 a)", "two")
        history_tip = in_history("rev-parse", "HEAD")
        path, warning = closing_merges(history, history_base, history_tip)
        assert [subject for _, subject in path] == ["Merge: one (W96)", "Merge: two (W97 a)"], path
        assert path[-1][0] == history_tip and [items_of(subject) for _, subject in path] == [["W96"], ["W97 a"]]
        assert warning is None, warning
        # A fast-forward of a branch with two commits: only the tip closes, and the check warns.
        in_history("switch", "-q", "-c", "three")
        in_history("commit", "-q", "--allow-empty", "-m", "First fix (W16)")
        in_history("commit", "-q", "--allow-empty", "-m", "Second fix (W17)")
        in_history("switch", "-q", "main")
        in_history("merge", "-q", "--ff-only", "three")
        fast_tip = in_history("rev-parse", "HEAD")
        path, warning = closing_merges(history, history_tip, fast_tip)
        assert path == [(fast_tip, "Second fix (W17)")], path
        assert warning and "--no-ff" in warning and fast_tip[:10] in warning, warning

        # The warning on fixed paths: none when a merge with an item is on the path, or when the checked commit is a merge.
        plain = ("p1p1p1p1p1pp", "0000", "A fix (W20)")
        merge_with_item = ("m5m5m5m5m5mm", "p1p1 e1e1", "Merge: five (W21)")
        merge_no_item = ("m6m6m6m6m6mm", "m5m5 f1f1", "Merge: no item")
        assert fast_forward_warning([plain, merge_with_item], "m5m5m5m5m5mm") is None
        assert fast_forward_warning([merge_with_item, plain], "p1p1p1p1p1pp") is None, "a merge with an item before a plain tip"
        assert fast_forward_warning([plain, merge_no_item], "m6m6m6m6m6mm") is None, "the tip is a merge"
        assert fast_forward_warning([merge_no_item, plain], "p1p1p1p1p1pp"), "no merge with an item, and a plain tip"
        assert fast_forward_warning([plain], "p1p1p1p1p1pp"), "a fast-forward of one commit"

        # D183: no stage writes to E: in a round with no game stage. The stage lists of the tool pass the rule.
        for kind, repo, paths in (("plugin", plugin, ["tools/x.py"]), ("plugin", plugin, ["mod/MWSE/mods/nirender/main.lua"]),
                                  ("plugin", plugin, ["contract/x"]), ("mge", mge, ["xtask/src/main.rs"]), ("mge", mge, ["d3d8/x.rs"])):
            assert evidence_problem(stages_for(kind, repo, paths)) is None, (kind, paths)
        with_base = stages_for("plugin", plugin, ["host/src/material.rs"], tips={"plugin": "a" * 40, "mge": "b" * 40},
                               mwse_commit="c" * 40, commits={"plugin": "d" * 40, "mge": "e" * 40})
        assert BASE_STAGE in [name for name, _, _ in with_base] and evidence_problem(with_base) is None, "the base set is made in the round that holds the game"
        for writer in (["robocopy", "x", "E:\\nirender-keep\\x"], ["tool", "--out=E:/nirender-keep/x"], ["cp", "x", "/e/nirender-keep/x"],
                       ["cp", "x", "e:/nirender-keep"], ["cp", "x", "E:"], ["cp", "x", "E:\\"], ["cp", "x", "E:x.txt"]):
            assert evidence_problem([("checks", root, good), ("a copy", root, writer)]), writer
            assert evidence_problem([("a copy", root, writer), (INSTANCE_STAGE, root, good)]) is None, writer
        assert evidence_problem([("a copy", "E:\\nirender-keep", ["cp", "x", "y"])]), "a stage in a folder on E:"
        for harmless in (["cp", "x", "D:/e/x"], ["cp", "-e", "x"], ["cp", "notes/e/x", "C:/x/e:"], ["cp", "x", "--e:x"]):
            assert evidence_problem([("a copy", root, harmless)]) is None, harmless
        # The round refuses such a list before any take, and holds nothing.
        try:
            run_round(slots, [("a copy", root, ["cp", "x", "E:\\nirender-keep\\x"])], "merge_step: x 4444444444", round_log, True)
        except ValueError as error:
            assert "on E:" in str(error), error
        else:
            raise AssertionError("a round with no game stage ran a stage that writes to E:")
        assert holders() == [], holders()
        # W399: the cite check of a merge, report only. A tree of no kind runs none; a missing tool, a failing run and a
        # timeout each give one "not checked" line and no exception; a run that works gives its lines with "cites: ".
        assert cite_lines(None, "a", "b") == []
        missing = os.path.join(root, "no-check-cites.py")
        assert cite_lines("plugin", "a", "b", missing) == ["cites: not checked: no %s" % missing]
        tool = os.path.join(root, "check_cites.py")
        open(tool, "w").close()
        calls = []

        def ran(code, stdout, stderr=""):
            def run(command, **options):
                calls.append(command)
                return types.SimpleNamespace(returncode=code, stdout=stdout, stderr=stderr)
            return run

        said = cite_lines("mge", "1111111", "2222222", tool, ran(0, "cite changed by the merge (OK -> RECHECK): x\ncites of render-link: 9 checked, 1 changed\n"))
        assert said == ["cites: cite changed by the merge (OK -> RECHECK): x", "cites: cites of render-link: 9 checked, 1 changed"], said
        assert calls[-1][1:] == ["-I", tool, "--merge", "1111111", "2222222", "--repo", "render-link"], calls[-1]
        assert cite_lines("plugin", "a", "b", tool, ran(2, "check_cites: no commit a\n")) == ["cites: not checked (exit 2): check_cites: no commit a"]
        assert calls[-1][-1] == "nirender", calls[-1]

        def slow(command, **options):
            raise subprocess.TimeoutExpired(command, options["timeout"])

        assert cite_lines("plugin", "a", "b", tool, slow)[0].startswith("cites: not checked: Command"), "a timeout is a line"
        # W528 item 4: the close line takes check_cites' own frame map line; a check that did not run is NOT RUN.
        counted = cite_lines("mge", "1", "2", tool, ran(0, "cites of render-link: 9 checked, 1 changed\nframe map: 1 cites moved, steps touched: not marked\n"))
        assert frame_map_of(counted) == "frame map: 1 cites moved, steps touched: not marked", counted
        assert frame_map_of(cite_lines("plugin", "a", "b", missing)) == "frame map: NOT RUN (not checked: no %s)" % missing
        assert frame_map_of(said) == "frame map: NOT RUN (the cite check gave no frame map line)", "an older check_cites.py"
        assert frame_map_of(["cites: cite changed by the merge (OK -> MOVED): frame map: x"]).startswith("frame map: NOT RUN"), "only a line that starts with it"
    print("merge_step self-test: ok")


def main():
    if len(sys.argv) > 2 and sys.argv[1] == REQUIRE_TESTS:
        return require_tests(sys.argv[2:])
    if len(sys.argv) in (8, 9) and sys.argv[1] == HELPER_SOURCES:
        return keep_helper_sources(*sys.argv[2:])
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("repo", nargs="?")
    parser.add_argument("commit", nargs="?")
    parser.add_argument("--base", help="the tip of the shared branch before the merge: the diff from it decides the stages")
    parser.add_argument("--sibling-base", help="the commit of the sibling repo for the base set, an ancestor of its HEAD: for a base with no kept pair; without it, the sibling's commit that passed with --base")
    parser.add_argument("--baseline", default=BASELINE, help="the folder that the gate compares with; without it, the accepted folder")
    parser.add_argument("--mwse", help="the MWSE commit of the round's sets; without it, the commit of the test installs' records (for a "
                        "plugin that needs an MWSE feature that the installs' MWSE lacks)")
    parser.add_argument("--no-wait", action="store_true", help="stop with exit code 3 where this would wait for the slot")
    parser.add_argument("--dry-run", action="store_true", help="print the stages: no slot, no build")
    parser.add_argument("--evidence", action="append", metavar="STAGE=FILE", help="the evidence file of a FORMAT CHANGE of the stage '%s': with it the stage passes, and the round's close line names the file's md5" % INSTANCE_STAGE)
    parser.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no build, no game")
    parser.add_argument("--open-deploys", action="store_true", help="list the WITHHELD deploys of %s that no deploy closed, and stop" % DEPLOYS_FILE)
    parser.add_argument("--gate-list", metavar="FILE", help="the Reviewer's passed list of the planned picture changes (W567 part D): the replay gate passes only when the "
                        "changed pictures equal it, and the round's last stage accepts them; its md5 goes into the close line")
    parser.add_argument("--quick", action="store_true", help="the fast stages of a worker's own tree (<repo>, else the current folder) at its HEAD, and one line for the done report (W567)")
    parser.add_argument("--sibling", help="with --quick: the tree of the other repo; without it, the tree beside <repo>")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.open_deploys:
        own = main_checkout()
        for line in open_deploy_lines(deploys_open(own, sibling_of("plugin", own))):
            say(line)
        return 0
    if args.quick:
        if args.commit or args.baseline != BASELINE or args.sibling_base or args.mwse or args.evidence or args.gate_list:
            parser.error("--quick takes [<tree>], --sibling, --base, --no-wait and --dry-run: it checks the tree's HEAD, with no set and no gate")
        return run_quick(args)
    if args.sibling:
        parser.error("--sibling is for --quick; a merge check takes the tree beside <repo>")
    if not args.repo or not args.commit or not args.base:
        parser.error("it needs <repo>, <commit> and --base <the tip before the merge>")
    evidence = evidence_of(args.evidence)
    if isinstance(evidence, str):
        say("FAIL the evidence: %s" % evidence)
        return 1
    EVIDENCE.clear()
    EVIDENCE.update(evidence)
    gate_list = gate_list_of(args.gate_list, args.baseline)
    if isinstance(gate_list, str):
        say("FAIL the gate list: %s" % gate_list)
        return 1
    GATE_LIST[:] = gate_list

    repo = os.path.abspath(args.repo)
    kind = repo_kind(repo)
    commit = git(repo, "rev-parse", args.commit)
    base = git(repo, "rev-parse", args.base)
    branch = branch_of(repo)
    tip_before = resolve(repo, branch + "@{1}") if branch else None
    passed_ref = "refs/merge-step/%s" % branch
    last_passed = resolve(repo, passed_ref) if branch else None
    problem = base_problem(base, commit, is_ancestor(repo, base, commit), tip_before, bool(tip_before) and is_ancestor(repo, base, tip_before),
                           last_passed, bool(last_passed) and is_ancestor(repo, base, last_passed))
    if problem:
        say("FAIL the base: %s" % problem)
        return 1
    paths = changed_paths(repo, base, commit)
    baseline = os.path.abspath(args.baseline)
    commits = under_test(kind, commit, resolve(sibling_of(kind, repo), "HEAD") if kind and os.path.isdir(sibling_of(kind, repo)) else None)
    if kind:
        say(helper_gate_paths(kind, repo, base)[1])
    what = "merge_step: %s %s" % (os.path.basename(repo), commit[:10])
    mwse_commit, mwse_why = round_mwse(args.mwse)
    stages, tips, why = planned_stages(kind, repo, paths, baseline, commits, base, args.sibling_base, mwse_commit, what, mwse_why, reuse_base=True) if kind else ([], None, None)
    if why:
        say("FAIL the tree: %s" % why)
        return 1
    sibling = sibling_of(kind, repo) if kind else ""
    has_sibling = os.path.isdir(sibling)
    sibling_changed = has_sibling and bool(git(sibling, "status", "--porcelain", "--untracked-files=no"))
    problem = tree_problem(kind, repo, stages, git(repo, "rev-parse", "HEAD"), commit, bool(git(repo, "status", "--porcelain", "--untracked-files=no")), sibling_changed)
    if problem:
        say("FAIL the tree: %s" % problem)
        return 1
    problem = baseline_problem(stages, baseline)
    if problem:
        say("FAIL the baseline: %s" % problem)
        return 1
    problem = evidence_problem(stages)
    if problem:
        say("FAIL the stages: %s" % problem)
        return 1
    for name in EVIDENCE:
        if name not in [stage for stage, _, _ in stages]:
            say("the evidence file of %s is not used: the round has no such stage" % name)
    if GATE_LIST:
        if any(name == ACCEPT_LIST_STAGE for name, _, _ in stages):
            say("the gate list %s (md5 %s): the replay gate passes these planned changes only, and the last stage accepts them" % GATE_LIST[0])
        else:
            say("the gate list %s is not used: the round has no replay gate" % GATE_LIST[0][0])
    sibling_hash = git(sibling, "rev-parse", "--short=10", "HEAD") if has_sibling else "none"
    sibling_head = resolve(sibling, "HEAD") if has_sibling else None
    sibling_base_note = ""
    if tips:
        sibling_base_note = " (base set at %s%s)" % (tips["mge" if kind == "plugin" else "plugin"][:10], ", --sibling-base" if args.sibling_base else ", the kept pair")
    elif args.sibling_base:
        sibling_base_note = " (--sibling-base unused: no base set in these stages)"
    say("%s %s, sibling %s%s, baseline %s: %d changed paths from %s; stages: %s" % (os.path.basename(repo), commit[:10], sibling_hash, sibling_base_note, baseline, len(paths), base[:10], ", ".join(name for name, _, _ in stages)))
    if any(name == SET_STAGE for name, _, _ in stages):
        say("the round's set: %s (MWSE %s, %s)" % (set_folder_of(round_set(commits, mwse_commit)), mwse_commit[:10], "--mwse" if args.mwse else "the installs' records"))
    if any(name == BASE_STAGE for name, _, _ in stages):
        for line in BASE_SET_NOTE:
            say(line)
    # The merges that a pass closes (D183), read before the round so that the warning of a fast-forward comes first.
    plugin_repo, mge_repo = (repo, sibling) if kind == "plugin" else (sibling, repo)
    for line in open_deploy_lines(deploys_open(plugin_repo, mge_repo)):
        say(line)
    merges, warning = closing_merges(repo, base, commit)
    if warning:
        say(warning)
    unread = unread_item_lines(merges)
    for line in unread:
        say(line)
    if args.dry_run:
        return 0

    slots = load_slots()
    log_path = log_path_of(commit)
    done = run_round(slots, stages, what, log_path, args.no_wait)
    if done is None:
        return 3
    results, summary = done
    say(summary)
    for line in NOT_DEPLOYED:
        say(line)
    # The cites of the Frame Map that the merge changes (D362): in the log too, and never a failure of the round.
    cites = cite_lines(kind, base, commit)
    with open(log_path, "a", encoding="utf-8") as log:
        for line in cites:
            say(line)
            log.write(line + "\n")
        # Said before the round; in the log too, beside the close that it explains.
        for line in unread + base_set_lines(stages):
            log.write(line + "\n")
    # close_after is the one gate of the closing lines: a failed stage or a pass against another folder writes none. A batch
    # of merges with one check closes the item of each merge on the first-parent path (D183).
    when = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
    # The same gate as the close: only a pass of the merge keeps an open deploy (W528).
    if NOT_DEPLOYED and results and all(results) and moves_the_ref(baseline):
        say(withhold_deploy(DEPLOYS_FILE, when, commits, NOT_DEPLOYED))
    still = open_deploy_lines(deploys_open(plugin_repo, mge_repo))
    written = close_after(results, baseline, merges,
                          lambda merges, stages: write_closes(CLOSES_FILE, when, os.path.basename(repo), merges, stages, log_path),
                          extra=[frame_map_of(cites)] + still[:1] + gate_list_close(stages) + base_set_lines(stages))
    if written:
        say("closes: %s in %s" % (", ".join(line.split("\t")[1] for line in written), CLOSES_FILE))
    elif written == []:
        say("closes: nothing to write (no new item in the subjects of %d merge(s): %s)" % (len(merges), "; ".join(subject for _, subject in merges)))
    return settle_refs(kind, repo, passed_ref, commit, sibling, sibling_head, stages, results, baseline, commits,
                       sibling_changed, log_path, mwse_commit)


if __name__ == "__main__":
    sys.exit(main())
