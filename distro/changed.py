#!/usr/bin/env python3
"""changed.py — which of Orbit's packages differ from the ones published.

    distro/changed.py PUBLISHED CRATE...

PUBLISHED is a listing of the repository's incoming folder (one
`name_version_arch.deb` a line); each version ends in the commit it was
built from (`+git<date>.<commit>`). A package is to be built again when
something it is made from changed between that commit and HEAD:

- a crate: its own folder, the workspace crates it depends on (all the way
  down), the files of data/, scripts/… they name (`../../data/x`), the
  crates.io packages it uses as Cargo.lock has them, and the workspace's
  Cargo.toml where it concerns them;
- the messages (data/i18n/*.ftl) are in every binary (orbit-app embeds
  them), but a crate counts as changed only when a message it uses
  changed: one of its prefix (`settings-…`) found in its sources, or any
  message whose prefix no source names;
- the metapackages (distro/meta/NAME), orbit-defaults and orbit-release,
  from their own files.

Also built: whatever is not published yet, or was built from a commit this
clone does not have. Prints one line per package to build, `NAME<TAB>why`;
the arm64 build of orbit-local-station is `orbit-local-station:arm64`.
"""

import json
import os
import re
import subprocess
import sys
import tomllib

ROOT = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip()
os.chdir(ROOT)

# Packages made from files rather than a crate: what they are made from.
SPECIAL = {
    "orbit-defaults": [
        "distro/defaults",
        "data/gsettings/90_orbit.gschema.override",
        "data/portals/orbit-portals.conf",
        "examples",
        ":(glob)data/io.github.julas23.Orbit.*.desktop",
    ],
    "orbit-release": ["distro/release", "data/brand/orbit-mark.svg", "LICENSE"],
}
I18N = "data/i18n"
DATA_REF = re.compile(r'"((?:\.\./)+)((?:data|scripts|distro|examples)/[^"*{}]+)')
COMMIT = re.compile(r"\+git\d{12}\.([0-9a-f]{7,40})")


def git(*args, check=True):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=check)


def differs(commit, paths):
    return bool(paths) and git("diff", "--quiet", commit, "HEAD", "--", *paths, check=False).returncode != 0


def published(listing):
    """name (or name:arm64) -> commit, from the listing's file names."""
    found = {}
    for line in listing:
        line = line.strip()
        if not line.endswith(".deb") or line.count("_") < 2:
            continue
        name, version, arch = line[:-4].split("_")[:3]
        key = f"{name}:arm64" if arch == "arm64" else name
        m = COMMIT.search(version)
        if m:
            found[key] = m.group(1)
    return found


def has_commit(commit):
    return git("cat-file", "-e", f"{commit}^{{commit}}", check=False).returncode == 0


def lock_at(commit):
    """name -> {(version, source)} of Cargo.lock at `commit`."""
    text = git("show", f"{commit}:Cargo.lock", check=False).stdout
    packages = {}
    for p in tomllib.loads(text).get("package", []) if text else []:
        packages.setdefault(p["name"], set()).add((p["version"], p.get("source", "")))
    return packages


def toml_at(commit):
    text = git("show", f"{commit}:Cargo.toml", check=False).stdout
    return tomllib.loads(text) if text else {}


def ftl_ids_changed(commit):
    """The message ids added, removed or changed in data/i18n since `commit`."""
    diff = git("diff", "-U0", commit, "HEAD", "--", I18N, check=False).stdout
    ids = set()
    for line in diff.splitlines():
        if line[:1] in "+-" and not line.startswith(("+++", "---")):
            m = re.match(r"[+-]([a-z0-9][a-z0-9-]*)\s*=", line)
            # A continuation line (an attribute, a long message): which
            # message it belongs to is unknown, so every one may be.
            ids.add(m.group(1) if m else "*")
    return ids


def main():
    listing_file, crates = sys.argv[1], sys.argv[2:]
    with open(listing_file) as f:
        have = published(f)
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked"], capture_output=True, text=True, check=True
        ).stdout
    )
    members = set(meta["workspace_members"])
    by_id = {p["id"]: p for p in meta["packages"]}
    ours = {by_id[i]["name"]: by_id[i] for i in members}
    deps_of = {n["id"]: [d["pkg"] for d in n["deps"]] for n in meta["resolve"]["nodes"]}

    def closure(name):
        """The crate's own id and every package it depends on, all the way."""
        seen, todo = set(), [ours[name]["id"]]
        while todo:
            i = todo.pop()
            if i in seen:
                continue
            seen.add(i)
            todo.extend(deps_of.get(i, []))
        return seen

    def crate_dir(pkg):
        return os.path.relpath(os.path.dirname(pkg["manifest_path"]), ROOT)

    def sources(dirs):
        for d in dirs:
            for base, subdirs, files in os.walk(d):
                subdirs[:] = [s for s in subdirs if s != "target"]
                for f in files:
                    path = os.path.join(base, f)
                    try:
                        with open(path, encoding="utf-8") as fh:
                            yield path, fh.read()
                    except (UnicodeDecodeError, OSError):
                        continue

    def data_refs(dirs):
        refs = set()
        for path, text in sources(dirs):
            for up, rest in DATA_REF.findall(text):
                ref = os.path.normpath(os.path.join(os.path.dirname(path), up + rest))
                if not ref.startswith(".."):
                    refs.add(ref)
        return refs

    # Every workspace source, once, for the messages' prefixes.
    all_text = {name: "\n".join(t for _, t in sources([crate_dir(p)])) for name, p in ours.items()}

    out = []
    for name in crates + ["orbit-local-station:arm64"] if "orbit-local-station" in crates else crates:
        crate = name.split(":")[0]
        commit = have.get(name)
        if commit is None:
            out.append((name, "not published"))
            continue
        if not has_commit(commit):
            out.append((name, f"published from {commit}, not in this clone"))
            continue
        ids = closure(crate)
        internal = [by_id[i] for i in ids if i in members]
        dirs = [crate_dir(p) for p in internal]
        refs = data_refs(dirs)
        why = None
        if differs(commit, dirs):
            why = "its sources"
        elif differs(commit, sorted(r for r in refs if not r.startswith(I18N))):
            why = "data it packs or embeds"
        if why is None:
            old, new = lock_at(commit), lock_at("HEAD")
            names = {by_id[i]["name"] for i in ids if i not in members}
            moved = sorted(n for n in names if old.get(n) != new.get(n))
            if moved:
                why = "Cargo.lock: " + ", ".join(moved[:5]) + ("…" if len(moved) > 5 else "")
        if why is None:
            before, now = toml_at(commit), toml_at("HEAD")
            ws_b, ws_n = before.get("workspace", {}), now.get("workspace", {})
            rest_b = {k: v for k, v in before.items() if k != "workspace"}
            rest_n = {k: v for k, v in now.items() if k != "workspace"}
            other_b = {k: v for k, v in ws_b.items() if k not in ("dependencies", "members")}
            other_n = {k: v for k, v in ws_n.items() if k not in ("dependencies", "members")}
            if rest_b != rest_n or other_b != other_n:
                why = "the workspace's Cargo.toml (profile, version)"
            else:
                d_b, d_n = ws_b.get("dependencies", {}), ws_n.get("dependencies", {})
                keys = {k for k in set(d_b) | set(d_n) if d_b.get(k) != d_n.get(k)}
                used = {d["name"] for p in internal for d in p["dependencies"]}
                hit = sorted(keys & used)
                if hit:
                    why = "workspace dependency " + ", ".join(hit)
        if why is None and any(r.startswith(I18N) for p in internal for r in data_refs([crate_dir(p)])):
            changed = ftl_ids_changed(commit)
            if changed:
                text = "\n".join(all_text[p["name"]] for p in internal)
                named = {i.split("-")[0] + "-" for i in changed if i != "*"}
                anywhere = "\n".join(all_text.values())
                unknown = "*" in changed or any(f'"{pre}' not in anywhere for pre in named)
                mine = sorted(pre for pre in named if f'"{pre}' in text)
                if mine:
                    why = "messages " + ", ".join(p + "…" for p in mine)
                elif unknown:
                    why = "messages no source names (to be safe)"
        if why:
            out.append((name, why))

    for name in sorted(os.listdir("distro/meta")):
        commit = have.get(name)
        if commit is None or not has_commit(commit):
            out.append((name, "not published"))
        elif differs(commit, [f"distro/meta/{name}"]):
            out.append((name, "its control fields"))
    for name, paths in SPECIAL.items():
        commit = have.get(name)
        if commit is None or not has_commit(commit):
            out.append((name, "not published"))
        elif differs(commit, paths):
            out.append((name, "its files"))
        elif name == "orbit-release" and toml_at(commit).get("workspace", {}).get("package", {}).get(
            "version"
        ) != toml_at("HEAD").get("workspace", {}).get("package", {}).get("version"):
            out.append((name, "the system's version"))

    for name, why in out:
        print(f"{name}\t{why}")


if __name__ == "__main__":
    main()
