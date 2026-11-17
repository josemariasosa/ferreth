# Reference Evidence Packs

Evidence packs collect bounded, locally available source material for architecture and mission work. They help offline reviewers distinguish what a source says from what Ferrum ETH has independently established. They are supporting evidence, not normative mission requirements or substitutes for validating the underlying source.

## Evidence labels

Each pack should distinguish:

- **Verified source facts:** statements directly supported by an identified source (and revision when available), with the scope of verification stated.
- **Source-authored claims:** statements made by the source's authors that have not been independently established.
- **Project interpretations:** Ferrum ETH's analysis or relevance assessment, clearly identified as interpretation.
- **Open questions:** issues the available material does not resolve.
- **Source identity and provenance:** source URLs and immutable revisions (for example, commit hashes) when available, along with access or preparation dates where useful.

Do not present a source-authored claim as an independently verified fact. Where a source is mutable and no immutable revision is available, say so and avoid implying that the evidence is frozen.

## Offline use and maintenance

An offline agent may use the contents as provided evidence, but must not imply that it checked the live source, current project status, or external links. It should mark conclusions that require live inspection as unresolved and report evidence limitations.

Preserve existing evidence packs when performing unrelated documentation work. Proposed corrections should be reported explicitly for review rather than silently editing a pack. No unavailable reference or revision should be fabricated.

## Current packs

- [Verified-zkEVM/evm-asm](evm-asm.md) — a public-documentation summary with source-authored status claims and stated limitations; no immutable upstream commit is recorded in the pack.
