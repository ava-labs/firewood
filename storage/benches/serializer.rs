// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::assigning_clones,
    reason = "Found 1 occurrences after enabling the lint."
)]
#![expect(
    clippy::unwrap_used,
    reason = "Found 7 occurrences after enabling the lint."
)]

use std::array::from_fn;
use std::fs::File;
use std::num::NonZeroU64;
use std::os::raw::c_int;

use bincode::Options;
use criterion::profiler::Profiler;
use criterion::{Criterion, criterion_group, criterion_main};
use firewood_storage::{LeafNode, Node, Path};
use pprof::ProfilerGuard;
use smallvec::SmallVec;

use std::path::Path as FsPath;

// For flamegraphs:
// cargo bench --bench serializer -- --profile-time=5

enum FlamegraphProfiler {
    Init(c_int),
    Active(ProfilerGuard<'static>),
}

fn file_error_panic<T, U>(path: &FsPath) -> impl FnOnce(T) -> U {
    |_| panic!("Error on file `{}`", path.display())
}

impl Profiler for FlamegraphProfiler {
    #[expect(clippy::unwrap_used)]
    fn start_profiling(&mut self, _benchmark_id: &str, _benchmark_dir: &FsPath) {
        if let Self::Init(frequency) = self {
            let guard = ProfilerGuard::new(*frequency).unwrap();
            *self = Self::Active(guard);
        }
    }

    #[expect(clippy::unwrap_used)]
    fn stop_profiling(&mut self, _benchmark_id: &str, benchmark_dir: &FsPath) {
        std::fs::create_dir_all(benchmark_dir).unwrap();
        let filename = "firewood-flamegraph.svg";
        let flamegraph_path = benchmark_dir.join(filename);
        let flamegraph_file =
            File::create(&flamegraph_path).unwrap_or_else(file_error_panic(&flamegraph_path));

        if let Self::Active(profiler) = self {
            profiler
                .report()
                .build()
                .unwrap()
                .flamegraph(flamegraph_file)
                .unwrap_or_else(file_error_panic(&flamegraph_path));
        }
    }
}

fn leaf(c: &mut Criterion) {
    let mut group = c.benchmark_group("leaf");
    let input = Node::Leaf(LeafNode {
        partial_path: Path(SmallVec::from_slice(&[0, 1])),
        value: Box::new([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
    });
    let serializer = bincode::DefaultOptions::new().with_varint_encoding();
    group.bench_with_input("serde", &input, |b, input| {
        b.iter(|| {
            serializer.serialize(input).unwrap();
        });
    });

    group.bench_with_input("manual", &input, |b, input| {
        b.iter(|| {
            let mut bytes = Vec::<u8>::new();
            input.as_bytes(0, &mut bytes);
        });
    });
    group.finish();
}

fn branch(c: &mut Criterion) {
    let mut group = c.benchmark_group("has_value");
    let mut input = Node::Branch(Box::new(firewood_storage::BranchNode {
        partial_path: Path(SmallVec::from_slice(&[0, 1])),
        value: Some(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9].into_boxed_slice()),
        children: from_fn(|i| {
            if i == 0 {
                Some(firewood_storage::Child::AddressWithHash(
                    NonZeroU64::new(1).unwrap(),
                    firewood_storage::HashType::from([0; 32]),
                ))
            } else {
                None
            }
        }),
    }));
    let serializer = bincode::DefaultOptions::new().with_varint_encoding();
    let serde_serializer = |b: &mut criterion::Bencher, input: &firewood_storage::Node| {
        b.iter(|| {
            serializer.serialize(input).unwrap();
        });
    };

    let manual_serializer = |b: &mut criterion::Bencher, input: &firewood_storage::Node| {
        b.iter(|| {
            let mut bytes = Vec::new();
            input.as_bytes(0, &mut bytes);
        });
    };

    group.bench_with_input("serde", &input, serde_serializer);
    group.bench_with_input("manual", &input, manual_serializer);
    group.finish();

    let mut group = c.benchmark_group("1_child");
    input.as_branch_mut().unwrap().value = None;
    group.bench_with_input("serde", &input, serde_serializer);
    group.bench_with_input("manual", &input, manual_serializer);
    let child = input.as_branch().unwrap().children[0].clone();
    group.finish();

    let mut group = c.benchmark_group("2_child");
    input.as_branch_mut().unwrap().children[1] = child.clone();
    group.bench_with_input("serde", &input, serde_serializer);
    group.bench_with_input("manual", &input, manual_serializer);
    group.finish();

    let mut group = c.benchmark_group("16_child");
    input.as_branch_mut().unwrap().children = std::array::from_fn(|_| child.clone());
    group.bench_with_input("serde", &input, serde_serializer);
    group.bench_with_input("manual", &input, manual_serializer);
    group.finish();
}

criterion_group!(
    name = serializers;
    config = Criterion::default().with_profiler(FlamegraphProfiler::Init(100));
    targets = leaf, branch
);
criterion_main!(serializers);
