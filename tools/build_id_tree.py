"""The build id names the work tree that was built, also when two work trees share a target
folder (W339).

The test makes a scratch repository with a workspace of two crates: `app`, a program whose
build script uses the repository's own `build_support/build_id.rs` as the build scripts here do,
and `dep`, a library that `app` uses. The scratch `.cargo/config.toml` gets the `[env]` table of
the repository's `.cargo/config.toml`. Two work trees A and B stand at two commits. They build
on one target folder in the order A, B, A, B, B, and each time the program prints its id.

PASS (exit 0): each build names the commit of its own tree, and the second build of B in a row
compiles nothing. FAIL (exit 1) otherwise. `--without-env` leaves the `[env]` table out: the
test must then FAIL (the build of B names the commit of A), which shows that it can see the
fault.

It runs cargo, so take a build place first. The scratch folder stays (its path is printed).

    python -B tools/build_id_tree.py [--repo DIR] [--work DIR] [--without-env]
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

BUILD_RS = '''#[path = "../build_support/build_id.rs"]
mod build_id;

fn main() {
    println!("cargo:rustc-env=BUILD_COMMIT={}", build_id::commit());
    build_id::watch_commit();
}
'''

FILES = {
    'Cargo.toml': '[workspace]\nmembers = ["app", "dep"]\nresolver = "2"\n',
    'app/Cargo.toml': '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\ndep = { path = "../dep" }\n',
    'app/build.rs': BUILD_RS,
    'app/src/main.rs': 'fn main() {\n    println!("{} {}", env!("BUILD_COMMIT"), dep::value());\n}\n',
    'dep/Cargo.toml': '[package]\nname = "dep"\nversion = "0.1.0"\nedition = "2021"\n',
    'dep/src/lib.rs': 'pub fn value() -> u32 {\n    7\n}\n',
    '.gitignore': 'target\n',
}

GIT = ['git', '-c', 'user.name=build-id-tree', '-c', 'user.email=build-id-tree@localhost']


def env_table(config_text):
    """The `[env]` table of a cargo config, with its header; empty when it has none."""
    lines = config_text.replace('\r\n', '\n').split('\n')
    table, inside = [], False
    for line in lines:
        if re.match(r'^\s*\[', line):
            inside = line.strip() == '[env]'
        if inside:
            table.append(line)
    return '\n'.join(table).strip() + '\n' if table else ''


def toolchain(repo):
    """The channel of the repository's rust-toolchain.toml, or None."""
    path = os.path.join(repo, 'rust-toolchain.toml')
    if not os.path.isfile(path):
        return None
    found = re.search(r'^channel\s*=\s*"([^"]+)"', open(path).read(), re.M)
    return found.group(1) if found else None


def run(args, cwd, env):
    result = subprocess.run(args, cwd=cwd, env=env, capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit('FAIL: %s in %s:\n%s%s' % (' '.join(args), cwd, result.stdout, result.stderr))
    return result.stdout.strip(), result.stderr


def write(root, name, text):
    path = os.path.join(root, name)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w', newline='\n') as out:
        out.write(text)


def main():
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--repo', default=here, help='the repository whose build_id.rs and [env] table are tested')
    parser.add_argument('--work', help='a new scratch folder (default: a new folder in the temp folder)')
    parser.add_argument('--without-env', action='store_true', help='leave the [env] table out: the test must FAIL')
    args = parser.parse_args()

    repo = os.path.abspath(args.repo)
    build_id = open(os.path.join(repo, 'build_support', 'build_id.rs')).read().replace('\r\n', '\n')
    table = env_table(open(os.path.join(repo, '.cargo', 'config.toml')).read())
    if not table and not args.without_env:
        sys.exit('FAIL: %s/.cargo/config.toml has no [env] table' % repo)
    if args.work:
        work = os.path.abspath(args.work)
        if os.path.exists(work):
            sys.exit('the work folder exists: %s' % work)
        os.makedirs(work)
    else:
        work = tempfile.mkdtemp(prefix='build-id-tree-')
    print('repository %s, scratch %s%s' % (repo, work, ', without the [env] table' if args.without_env else ''))

    env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
    channel = toolchain(repo)
    if channel:
        env['RUSTUP_TOOLCHAIN'] = channel

    main_tree = os.path.join(work, 'main')
    for name, text in FILES.items():
        write(main_tree, name, text)
    write(main_tree, 'build_support/build_id.rs', build_id)
    if not args.without_env:
        write(main_tree, '.cargo/config.toml', table)
    run(GIT + ['init', '-q'], main_tree, env)
    run(GIT + ['add', '.'], main_tree, env)
    run(GIT + ['commit', '-q', '-m', 'one'], main_tree, env)
    first, _ = run(GIT + ['rev-parse', '--short', 'HEAD'], main_tree, env)
    write(main_tree, 'notes.txt', 'two\n')
    run(GIT + ['add', '.'], main_tree, env)
    run(GIT + ['commit', '-q', '-m', 'two'], main_tree, env)
    second, _ = run(GIT + ['rev-parse', '--short', 'HEAD'], main_tree, env)
    commits = {'A': first, 'B': second}
    for name in ('A', 'B'):
        run(GIT + ['worktree', 'add', '-q', '--detach', os.path.join(work, name), commits[name]], main_tree, env)

    build_env = dict(env, CARGO_TARGET_DIR=os.path.join(work, 'target'))
    program = os.path.join(work, 'target', 'debug', 'app.exe' if os.name == 'nt' else 'app')
    passed = True
    order = ('A', 'B', 'A', 'B', 'B')
    for step, name in enumerate(order):
        tree = os.path.join(work, name)
        _, log = run(['cargo', 'build', '-v'], tree, build_env)
        compiled = [line.split()[1] for line in log.splitlines() if line.strip().startswith('Compiling ')]
        said, _ = run([program], tree, build_env)
        good = said.split()[0] == commits[name]
        again = step > 0 and order[step - 1] == name
        if again and compiled:
            good = False
        passed = passed and good
        print('build %d, tree %s (commit %s): the program says %s; compiled: %s%s' % (
            step + 1, name, commits[name], said.split()[0], ', '.join(compiled) or 'nothing',
            '' if good else '  <- WRONG'))
    print('PASS' if passed else 'FAIL')
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
