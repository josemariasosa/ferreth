"""Copy the two private encoder functions verbatim into a standalone probe."""
from pathlib import Path
import re
import sys

repo, output = map(Path, sys.argv[1:])
modules = []
for module, error in (("compiler", "CompileError"), ("orchestrator", "OrchestratorError")):
    source = (repo / f"crates/ferreth-compiler/src/{module}.rs").read_text()
    function = re.search(r"^fn encode_jal_zero\(.*?^}", source, re.M | re.S).group()
    modules.append(
        f"mod {module}_encoder {{\n"
        f"    #[derive(Debug)]\n    #[allow(dead_code)]\n"
        f"    pub enum {error} {{ Link {{ description: String }} }}\n"
        f"    pub(super) use self::inner::encode;\n"
        f"    mod inner {{\n        use super::{error};\n"
        f"{function}\n"
        f"        pub(crate) fn encode(offset: i64) -> Result<u32, {error}> {{\n"
        "            encode_jal_zero(offset)\n        }\n    }\n}\n"
    )
output.write_text("\n".join(modules))
