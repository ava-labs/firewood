use futures::executor::block_on;
use growthring::{
    wal::{WalBytes, WalLoader, WalRingId, WalWriter},
    walerror::WalError,
    WalStoreAio,
};
use rand::{seq::SliceRandom, Rng, SeedableRng};

fn test(records: Vec<String>, wal: &mut WalWriter<WalStoreAio>) -> Vec<WalRingId> {
    let mut res = Vec::new();
    for r in wal.grow(records).into_iter() {
        let ring_id = futures::executor::block_on(r).unwrap().1;
        println!("got ring id: {:?}", ring_id);
        res.push(ring_id);
    }
    res
}

fn recover(payload: WalBytes, ringid: WalRingId) -> Result<(), WalError> {
    println!(
        "recover(payload={}, ringid={:?}",
        std::str::from_utf8(&payload).unwrap(),
        ringid
    );
    Ok(())
}

fn main() {
    let wal_dir = "./wal_demo1";
    let mut rng = rand::rngs::StdRng::seed_from_u64(0);
    let mut loader = WalLoader::new();
    loader.file_nbit(9).block_nbit(8);

    let store = WalStoreAio::new(wal_dir, true, None).unwrap();
    let mut wal = block_on(loader.load(store, recover, 0)).unwrap();
    for _ in 0..3 {
        test(
            ["hi", "hello", "lol"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>(),
            &mut wal,
        );
    }
    for _ in 0..3 {
        test(
            vec!["a".repeat(10), "b".repeat(100), "c".repeat(1000)],
            &mut wal,
        );
    }

    let store = WalStoreAio::new(wal_dir, false, None).unwrap();
    let mut wal = block_on(loader.load(store, recover, 0)).unwrap();
    for _ in 0..3 {
        test(
            vec![
                "a".repeat(10),
                "b".repeat(100),
                "c".repeat(300),
                "d".repeat(400),
            ],
            &mut wal,
        );
    }

    let store = WalStoreAio::new(wal_dir, false, None).unwrap();
    let mut wal = block_on(loader.load(store, recover, 100)).unwrap();
    let mut history = std::collections::VecDeque::new();
    for _ in 0..3 {
        let mut ids = Vec::new();
        for _ in 0..3 {
            let mut records = Vec::new();
            for _ in 0..100 {
                let rec = "a".repeat(rng.gen_range(1..1000));
                history.push_back(rec.clone());
                if history.len() > 100 {
                    history.pop_front();
                }
                records.push(rec)
            }
            for id in test(records, &mut wal).iter() {
                ids.push(*id)
            }
        }
        ids.shuffle(&mut rng);
        for e in ids.chunks(20) {
            println!("peel(20)");
            futures::executor::block_on(wal.peel(e, 100)).unwrap();
        }
    }
    for (rec, ans) in
        block_on(wal.read_recent_records(100, &growthring::wal::RecoverPolicy::Strict))
            .unwrap()
            .into_iter()
            .zip(history.into_iter().rev())
    {
        assert_eq!(std::str::from_utf8(&rec).unwrap(), &ans);
        println!("{}", std::str::from_utf8(&rec).unwrap());
    }
}
