use firewood_macros::hash_mode;
use test_case::test_case;

trait HashMode {}
struct EthHash;
struct MerkleDbHash;
impl HashMode for EthHash {}
impl HashMode for MerkleDbHash {}

#[hash_mode]
#[test_case(1, 2; "small")]
#[test_case(10, 20; "large")]
fn adds<H: HashMode>(a: u32, b: u32) {
    assert!(a < b);
}

fn main() {}
