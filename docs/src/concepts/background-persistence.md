# Background Persistence

Commits do not write to disk on the caller's thread. A background thread owned
by the `PersistWorker` in the `firewood` crate persists revisions, and
`max_persistence_gap` bounds how many commits may be outstanding before it must
run. Each commit takes a permit; when the remaining permits fall to the
threshold, the background thread persists the latest revision and releases the
permits. On close, the last committed revision is always persisted.

The sequence below uses `max_persistence_gap = 10`:

```mermaid
sequenceDiagram
    participant Caller
    participant Main as Main Thread
    participant BG as Background Thread
    participant Disk

    Note over Main: permits = 10

    loop Commits 1-4
        Caller->>Main: commit()
        Main->>Main: store latest revision, permits -= 1
        Note right of BG: Sleeping (permits > threshold)
    end

    Note over Main: permits = 6

    Caller->>Main: commit() (5th)
    Main->>Main: store latest revision, permits -= 1
    Note over Main: permits = 5 = threshold
    Main->>BG: notify persist_ready
    BG->>Disk: persist latest revision
    BG->>BG: release 5 permits
    Note over Main: permits = 10

    loop Commits 6-8
        Caller->>Main: commit()
        Main->>Main: store latest revision, permits -= 1
        Note right of BG: Sleeping (permits > threshold)
    end

    Caller->>Main: close()
    Main->>BG: shutdown signal
    BG->>Disk: persist last committed revision
    Note right of Disk: Latest committed revision is persisted
```
