use pdb::FallibleIterator;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let mut pdb = pdb::PDB::open(std::fs::File::open(path).unwrap()).unwrap();
    let map = pdb.address_map().unwrap();
    let dbi = pdb.debug_information().unwrap();
    let mut modules = dbi.modules().unwrap();
    while let Some(module) = modules.next().unwrap() {
        let Some(info) = pdb.module_info(&module).unwrap() else { continue };
        let mut syms = info.symbols().unwrap();
        while let Some(sym) = syms.next().unwrap() {
            if let Ok(pdb::SymbolData::Procedure(p)) = sym.parse() {
                if let Some(rva) = p.offset.to_rva(&map) {
                    println!("{:08x} {:x} {}", rva.0 + 0x10000, p.len, p.name.to_string());
                }
            }
        }
    }
}
