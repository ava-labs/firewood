use firewood_macros::hash_mode;

trait HashMode {}
struct EthHash;
struct MerkleDbHash;
impl HashMode for EthHash {}
impl HashMode for MerkleDbHash {}

#[hash_mode]
#[allow(dead_code)]
fn runs_under_both<H: HashMode>() {
    let _ = core::marker::PhantomData::<H>;
}

fn main() {}
