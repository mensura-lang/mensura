/-
Axiom hygiene gate (ADR 0021,
`docs/decisions/0021-formal-proof-pipeline.md`): every declaration in the
`Mensura` namespace may depend only on the three standard axioms.  CI runs
this file (`lake env lean AxiomCheck.lean`); elaboration fails if any other
axiom is reachable, so a smuggled `axiom` fails the pull request.
-/

import Lean.Util.CollectAxioms
import Mensura

open Lean

def allowedAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

open Elab Command in
elab "#axiom_gate" : command => do
  let env ← getEnv
  let mut offenders : Array (Name × Name) := #[]
  let mut checked := 0
  for (name, _) in env.constants.toList do
    unless (`Mensura).isPrefixOf name do continue
    checked := checked + 1
    let axioms ← liftCoreM (collectAxioms name)
    for ax in axioms do
      unless allowedAxioms.contains ax do
        offenders := offenders.push (name, ax)
  if checked = 0 then
    throwError "axiom gate scanned no declarations; is Mensura imported?"
  unless offenders.isEmpty do
    let lines := offenders.map (fun (n, ax) => s!"  {n} uses {ax}")
    throwError "declarations depending on non-standard axioms:\n{String.intercalate "\n" lines.toList}"
  logInfo s!"axiom gate passed: {checked} declarations, only standard axioms"

#axiom_gate
