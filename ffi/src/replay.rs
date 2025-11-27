// we want to record some essential operations that our ffi users call, we have an outside benchmark,
// we record the ops here, we want to replay later, everything should be gated with block-replay feature
// we will add some metadata in future
// all (de)serialization should be done with rkyv, feel free to change struct contents.
// start simple, extend later.

// some structs depend on latest revision of dataset and input is clear, like get latest,
// or get from root

// some structs like propose/commit/get_from_proposal, are like a method on a handle
// they need a handle id.

// some ops return handles

// the way I want to handle these are to maintain a hashmap [handle pointer => u64 incremental id]
// when replaying, I'll just make sure I'm matching ids correctly.

pub struct GetLatest {
    key: Box<[u8]>,
}

pub struct GetFromProposal {
    proposal_id: u64,
    key: Box<[u8]>,
}

pub struct GetFromRoot {
    root: Box<[u8]>,
    key: Box<[u8]>,
}

pub struct Batch {
    pairs: Vec<(Box<[u8]>, Box<[u8]>)>,
}

// propose on db / propose on proposal, different types?

pub struct ProposeOnDB {
    pairs: Vec<(Box<[u8]>, Box<[u8]>)>,
    returned_proposal_id: u64,
}

pub struct ProposeOnProposal {
    proposal_id: u64,
    pairs: Vec<(Box<[u8]>, Box<[u8]>)>,
    returned_proposal_id: u64,
}

pub struct Commit {
    proposal_id: u64,
    returned_hash: Box<[u8]>,
}

pub(crate) enum DbOperation {
    GetLatest(GetLatest),
    GetFromProposal(GetFromProposal),
    GetFromRoot(GetFromRoot),
    Batch(Batch),
    ProposeOnDB(ProposeOnDB),
    ProposeOnProposal(ProposeOnProposal),
    Commit(Commit),
}