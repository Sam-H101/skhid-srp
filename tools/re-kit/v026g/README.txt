Survarium v0.26g executable labels

Condensed game and engine function information for inspecting and patching
v0.26g build 2777. Names come from comparison with the v0.21d EXE/PDB.
Vendor-library entries are omitted.

Files
  functions.json       Qualified names mapped to lists of preferred VAs.
  functions.info.json  VA/RVA, file offset, entry bytes, name provenance and caveats.
  abi.json             Target register/stack inputs, local RET cleanup and notes.
  identity.json        Exact executable/PDB hashes, image bases and PE sections.
  lookup.py            Search names and display their addresses and ABI.

Usage (Python 3, from the repository root)
  python tools/re-kit/v026g/lookup.py stage_screen_image
  python tools/re-kit/v026g/lookup.py 'vostok::render::ui::renderer::draw_text' --exact
  python tools/re-kit/v026g/lookup.py draw_triangles --runtime-base 0x400000

Supply --runtime-base only when the loaded module base is known. Preferred
VAs use image base 0x10000; RVA = preferred VA - image base, and runtime VA =
loaded base + RVA. Use file_offset for on-disk bytes. The lookup does not
validate a running process or executable against identity.json.

Data conventions
Names map to address lists because overloads and multiple bodies can share
a PDB name. The information and ABI files are keyed by preferred VA. Several
names can share one body; keep the qualified name when choosing an ABI entry.

LTO changes calling conventions. Use the documented target input registers
and entry-stack locations, not a source prototype. Multiple ABI contracts
can provide complementary observations. complete_contract=false means some
arguments, clobbers, results or other behavior remain unestablished. Automated
observations identify incoming machine values without assigning semantic roles.
An empty local RET list does not imply zero cleanup; a tail jump can transfer
that responsibility to another function. Reviewed tail jumps are not exhaustive.
Entry bytes and heuristic boundaries do not establish a safe patch length.
Read per-function notes, unknowns and evidence_caveats before relying on a label.

Detailed evidence, reviews, unresolved candidates, binaries and audit history
are kept separately in private/function-map/. These condensed files can be
queried without that archive. Reusable analysis tools live in ../common/.
