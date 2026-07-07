# Tooling for the formal development (ADR 0021).  Run from formal/.
#
# Python dependencies (plasTeX and the leanblueprint plugins) are declared
# in blueprint/pyproject.toml and resolved by uv on first use; no install
# step is needed.  The leanblueprint CLI itself assumes the lakefile sits
# at the git root, which does not fit this nested layout, so the render
# targets drive plastex/latexmk directly.

BLUEPRINT := blueprint

.PHONY: help build axioms color web pdf

help:
	@echo "make build   -- lake build the Lean development"
	@echo "make axioms  -- run the axiom gate (AxiomCheck.lean)"
	@echo "make color   -- derive blueprint node colors from the code"
	@echo "make web     -- render the blueprint dashboard to blueprint/web/"
	@echo "make pdf     -- build the blueprint PDF in blueprint/print/"

build:
	lake build

axioms:
	lake env lean AxiomCheck.lean

color:
	uv run $(BLUEPRINT)/color.py

web: color
	cd $(BLUEPRINT)/src && uv run --project .. plastex -c plastex.cfg web.tex

pdf: color
	cd $(BLUEPRINT)/src && latexmk -output-directory=../print
