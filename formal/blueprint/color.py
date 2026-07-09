#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Color the blueprint from the REAL state of the Lean code (ADR 0021).

Mechanical, no intelligence: for each node `\\lean{Name}` in
`blueprint/src/content.tex`, decide the color from the declaration's
existence and sorry-freedom:

    declaration absent           -> WHITE  (remove \\leanok; planned)
    declared but with sorry      -> BLUE   (\\leanok on the statement only)
    declared and sorry-free      -> GREEN  (\\leanok on statement and proof)

It never touches the STRUCTURE (nodes, statements, edges); it only manages
the `\\leanok` token, so hand-written status cannot drift from the code.
Adapted from the esquadro-color script of the spec-driven-lean framework.

Two modes:

    color.py          rewrite content.tex in place.  Fail-safe: any error
                      exits 0 so a hook using it never blocks a commit.
    color.py --check  change nothing; exit 1 when the committed file is out
                      of sync with the code, when a `\\lean{...}` name
                      neither resolves to a declaration nor carries a
                      `% planned` comment on its line, when a `% planned`
                      marker is stale (the declaration exists), or when a
                      `\\uses{...}` edge names a label that does not exist.
                      Exceptions propagate: a gate that cannot fail is
                      useless.  CI runs this mode; it needs only the
                      standard library (`python3 color.py --check`).

A planned (white) node marks the line bearing its `\\lean{...}` with the
comment `% planned`; remove the marker once the result lands in Lean.

Run from anywhere: paths are resolved relative to this file, with the Lean
sources in the parent directory (`formal/`).
"""
import difflib
import re
import sys
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
LEAN_ROOT = HERE.parent
CONTENT = HERE / "src" / "content.tex"

DECL_RE = re.compile(
    r"^\s*(?:@\[[^\]]*\]\s*)*(?:noncomputable\s+|private\s+|protected\s+|scoped\s+|local\s+)*"
    r"(?:def|theorem|lemma|instance|abbrev|structure|inductive)\s+([A-Za-z0-9_'.]+)"
)
NS_RE = re.compile(r"^\s*namespace\s+([A-Za-z0-9_'.]+)")
END_RE = re.compile(r"^\s*end\s+([A-Za-z0-9_'.]+)")
SORRY_RE = re.compile(r"\b(sorry|admit)\b")

LEAN_RE = re.compile(r"\\lean\{([^}]*)\}")
PROOF_RE = re.compile(r"\\begin\{proof\}")
PLANNED_RE = re.compile(r"(?<!\\)%\s*planned\b")
LABEL_RE = re.compile(r"\\label\{([^}]*)\}")
USES_RE = re.compile(r"\\uses\{([^}]*)\}")
COMMENT_RE = re.compile(r"(?<!\\)%")


def strip_comment(line):
    """Drop the TeX comment part of a line (an unescaped `%` to the end)."""
    m = COMMENT_RE.search(line)
    return line[: m.start()] if m else line


def scan_decls():
    """full-name -> has_sorry (sorry-free wins if a name is duplicated)."""
    status = {}
    for lean in LEAN_ROOT.rglob("*.lean"):
        if ".lake" in lean.parts:
            continue
        try:
            lines = lean.read_text(encoding="utf-8", errors="ignore").splitlines()
        except OSError:
            continue
        ns = []
        starts = []
        for i, line in enumerate(lines):
            mn = NS_RE.match(line)
            if mn:
                ns.append(mn.group(1))
                continue
            me = END_RE.match(line)
            if me and ns and ns[-1] == me.group(1):
                ns.pop()
                continue
            md = DECL_RE.match(line)
            if md:
                full = ".".join(ns + [md.group(1)]) if ns else md.group(1)
                starts.append((i, full))
        for idx, (line_no, full) in enumerate(starts):
            end = starts[idx + 1][0] if idx + 1 < len(starts) else len(lines)
            body = "\n".join(lines[line_no:end])
            has_sorry = bool(SORRY_RE.search(body))
            status[full] = status.get(full, True) and has_sorry
    return status


def set_token(line, token, present):
    """Ensure (or remove) `token` on the line, without duplicating it.

    Insertion goes before any trailing `%` comment, so a stale
    `% planned` marker can never comment the token out.
    """
    has = token in line
    if present and not has:
        body = line.rstrip("\n")
        mc = re.search(r"(?<!\\)%", body)
        if mc:
            return body[: mc.start()].rstrip() + token + "  " + body[mc.start() :] + "\n"
        return body + token + "\n"
    if not present and has:
        return line.replace(token, "")
    return line


def recolor(lines, status):
    """Pure recoloring: return (new_lines, nodes) without touching disk.

    `nodes` records, per `\\lean{...}` line, the 1-based line number, the
    Lean name, whether it resolves to a declaration, and whether the line
    carries a `% planned` marker.
    """
    out = list(lines)
    nodes = []
    i = 0
    while i < len(out):
        m = LEAN_RE.search(strip_comment(out[i]))
        if not m:
            i += 1
            continue
        name = m.group(1).strip()
        declared = name in status
        green = declared and not status[name]
        nodes.append((i + 1, name, declared, bool(PLANNED_RE.search(out[i]))))
        # Statement: \leanok when declared (blue or green); removed when white.
        out[i] = set_token(out[i], r"\leanok", declared)
        # Proof: find this node's \begin{proof} (before the next \lean).
        j = i + 1
        while j < len(out) and not LEAN_RE.search(strip_comment(out[j])):
            if PROOF_RE.search(strip_comment(out[j])):
                out[j] = set_token(out[j], r"\leanok", green)
                break
            j += 1
        i += 1
    return out, nodes


def dangling_edges(lines):
    """Every \\uses target must exist as a \\label in the file."""
    text = "\n".join(strip_comment(line.rstrip("\n")) for line in lines)
    labels = {m.group(1).strip() for m in LABEL_RE.finditer(text)}
    errors = []
    for m in USES_RE.finditer(text):
        for target in m.group(1).split(","):
            t = target.strip()
            if t and t not in labels:
                errors.append("\\uses edge names unknown label '%s'" % t)
    return errors


def check():
    status = scan_decls()
    disk = CONTENT.read_text(encoding="utf-8").splitlines(keepends=True)
    derived, nodes = recolor(disk, status)
    errors = []
    if derived != disk:
        sys.stdout.writelines(
            difflib.unified_diff(
                disk,
                derived,
                fromfile="content.tex (committed)",
                tofile="content.tex (derived from Lean)",
            )
        )
        errors.append("\\leanok out of sync with the code; run `make color` and commit")
    for line_no, name, declared, planned in nodes:
        if not declared and not planned:
            errors.append(
                "line %d: \\lean{%s} does not resolve to any Lean declaration"
                " (mark the line with '%% planned' if intended)" % (line_no, name)
            )
        if declared and planned:
            errors.append(
                "line %d: stale '%% planned' marker: %s is declared" % (line_no, name)
            )
    errors.extend(dangling_edges(disk))
    for e in errors:
        print("blueprint check: %s" % e, file=sys.stderr)
    return 1 if errors else 0


def main():
    if not CONTENT.exists():
        return 0
    status = scan_decls()
    lines = CONTENT.read_text(encoding="utf-8", errors="ignore").splitlines(keepends=True)
    new, _ = recolor(lines, status)
    CONTENT.write_text("".join(new), encoding="utf-8")
    return 0


if __name__ == "__main__":
    if "--check" in sys.argv[1:]:
        sys.exit(check())
    try:
        sys.exit(main())
    except Exception:
        sys.exit(0)
