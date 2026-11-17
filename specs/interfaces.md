# Hardware Interface Conventions

| Field | Value |
|---|---|
| Document type | Initial interface-design guidance |
| Version | 0.1 |
| Status | Proposed |
| Scope | Hardware module contracts in Ferrum ETH |

This document defines general conventions, not a module specification. An approved executable mission owns the exact interface and acceptance criteria for its hardware task. For ETH-001, specific port declarations, operation encodings, and invalid-selector behavior remain provisional until that mission is approved.

## Conventions

### Widths and values

- Declare every signal width explicitly; avoid relying on implicit sizing or unsized constants in width-sensitive expressions.
- Document the meaning and legal range of each field, including whether values are bit patterns, booleans, or encoded selections.
- State truncation, extension, carry, and overflow behavior where relevant. Do not rely on simulator-specific sizing.
- Use a declared bit-vector width for operands and results; ETH-001's abstract arithmetic values are 256 bits.

### Signal naming

- Use descriptive, consistent lower-case names with underscores where needed.
- Name signals for their function and direction at the module boundary; document any repository-specific suffix convention in the mission before implementation.
- Keep names distinct from language keywords and avoid names that imply unsupported behavior, such as a `ready` signal without a defined handshake contract.

### Operations and selectors

- Document each operation selector's width, encoding, supported operations, and reserved or invalid values.
- Use named constants or an enumerated type where supported by the selected SystemVerilog subset; do not make behavior depend on undocumented numeric encodings.
- Define the output behavior for every selector value. A mission must specify whether unsupported encodings produce a defined result, an error indication, or are excluded by an explicit input assumption.

### Signed and unsigned interpretation

- Treat a signal as a bit vector unless its contract explicitly assigns signed meaning.
- Define signed interpretation and width whenever signed arithmetic or comparison is required; unsigned operations must be identified as such.
- Do not infer signedness from a signal name or host-language type. For two's-complement operations, specify the exact width.

### Combinational and registered boundaries

- Declare whether a module is combinational or sequential and identify all state boundaries.
- A combinational contract defines outputs as functions of current inputs, without clock, reset, request, or validity behavior unless explicitly added.
- A registered contract must define clocking, reset (if any), acceptance, output-valid behavior, latency, and behavior for overlapping requests or backpressure.
- Do not add a clock or reset to a combinational milestone merely as a presumed future requirement.

### Defined outputs and evolution

- Define outputs for all legal inputs and specify assumptions or behavior for invalid inputs.
- Avoid inferred latches in combinational logic; all output paths must follow the contract.
- Preserve existing interface meaning across compatible revisions. A breaking change requires an explicit version or migration note and updated dependent missions and tests; never silently reuse a field or selector with a changed meaning.
- Keep module-level computation separate from system-level responsibilities such as EVM stack handling, gas accounting, memory, or processor control unless a mission explicitly assigns them.
