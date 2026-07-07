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

Run from anywhere: paths are resolved relative to this file, with the Lean
sources in the parent directory (`formal/`).  Fail-safe: any error exits 0
so a hook using it never blocks a commit.
"""
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
    """Ensure (or remove) `token` on the line, without duplicating it."""
    has = token in line
    if present and not has:
        return line.rstrip("\n") + token + "\n"
    if not present and has:
        return line.replace(token, "")
    return line


def main():
    if not CONTENT.exists():
        return 0
    status = scan_decls()
    lines = CONTENT.read_text(encoding="utf-8", errors="ignore").splitlines(keepends=True)
    LEAN_RE = re.compile(r"\\lean\{([^}]*)\}")
    PROOF_RE = re.compile(r"\\begin\{proof\}")

    i = 0
    while i < len(lines):
        m = LEAN_RE.search(lines[i])
        if not m:
            i += 1
            continue
        name = m.group(1).strip()
        declared = name in status
        green = declared and not status[name]
        # Statement: \leanok when declared (blue or green); removed when white.
        lines[i] = set_token(lines[i], r"\leanok", declared)
        # Proof: find this node's \begin{proof} (before the next \lean).
        j = i + 1
        while j < len(lines) and not LEAN_RE.search(lines[j]):
            if PROOF_RE.search(lines[j]):
                lines[j] = set_token(lines[j], r"\leanok", green)
                break
            j += 1
        i += 1

    CONTENT.write_text("".join(lines), encoding="utf-8")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        sys.exit(0)
