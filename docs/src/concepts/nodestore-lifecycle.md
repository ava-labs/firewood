# NodeStore Lifecycle

A `NodeStore<T, S, H>` in `firewood-storage` holds one revision of the trie. The
state type `T` records what can be done with that revision:

- `Committed`: a committed revision with no in-memory changes.
- `Mutable<Propose>`: a proposal still being modified. It holds in-memory nodes,
  a delete list, and a parent.
- `Arc<ImmutableProposal>`: a proposal that has been hashed and assigned
  addresses.
- `Mutable<Recon>`: a reconstruction workspace being modified. It holds
  in-memory nodes but no delete list or parent.
- `Reconstructed`: a read-only view produced by reconstruction, which can be
  reconstructed again.

Red arrows consume their source; blue arrows borrow it.

```mermaid
flowchart TD

subgraph committed["Committed Revisions"]
  L("Latest Committed")
  O("Oldest Committed")
end

O -->|"PersistWorker::reap"| E(("Expired"))

L -->|"Db::propose"| MP("Mutable< Propose >")
MP --> IP("Arc< ImmutableProposal >")
IP -->|"Proposal::commit"| C2("New Committed")
IP -->|"Proposal::propose"| MP

committed -->|"reconstruct"| MR("Mutable< Recon >")
R -->|"reconstruct"| MR
MR --> R("Reconstructed")

linkStyle 0 stroke:red
linkStyle 1 stroke:blue
linkStyle 2 stroke:red
linkStyle 3 stroke:red
linkStyle 4 stroke:blue
linkStyle 5 stroke:blue
linkStyle 6 stroke:red
linkStyle 7 stroke:red

style E color:#FFFFFF, fill:#AA00FF, stroke:#AA00FF
style MP fill:#E8F5E9
style IP fill:#E8F5E9
style MR fill:#E3F2FD
style R fill:#E3F2FD
```

The step-by-step proposal and reconstruction lifecycles, with links to each
conversion, are in the
[`NodeStore` rustdoc](https://ava-labs.github.io/firewood/rustdoc/firewood_storage/nodestore/struct.NodeStore.html).
