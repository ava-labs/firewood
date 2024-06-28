= The storage layer

Each of these structures contain a NodeStore

== Committed

* Represents a committed revision
* Is immutable
* Has a different root\_address
* The free_list is never referenced
* Maintains a list of addresses that can be freed
* Parented by either another Committed or FileBacked

== Proposed

* Represents a proposal for a new revision
* Can be mutable or immutable
* Mutable revisions may have incomplete hash information
* Has a (probably different from its parent) root\_address and a free list
* Parented by another Proposed or FileBacked


        +-----------+
        | Committed |
        +-----------+
              |
              v
        +-----------+
        | Committed |
        +-----------+
            /   \
           v     v
  +----------+ +----------+
  | Proposed | | Proposed |
  +----------+ +----------+
       |
       v
  +----------+
  | Proposed |
  +----------+

