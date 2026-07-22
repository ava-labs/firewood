use firewood_macros::hash_mode;

trait HashMode {}
struct EthHash;
struct MerkleDbHash;
impl HashMode for EthHash {}
impl HashMode for MerkleDbHash {}

#[hash_mode]
fn t<H: HashMode>((a, b): (u32, u32)) {
    let _ = (a, b);
}

fn main() {}
