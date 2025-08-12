// use std::sync::Arc;
// use crate::merkle::Key;
// use crate::stream::MerkleKeyValueStream;
// use firewood_storage::{NodeStore, ReadableStorage, TrieReader};
//
// pub struct KeyValueIterator {
//
// }
//
// pub trait KVIterator {
//     fn key_value_iter_from_key<K: AsRef<[u8]>>(
//         &self,
//         key: K,
//     ) -> MerkleKeyValueStream<'_, T>;
// }
//
// impl<T,S> KVIterator for NodeStore<T, S>
// where
//
//     NodeStore<T, S>: TrieReader,
//     T: std::fmt::Debug,
//     S: ReadableStorage,
// {
//     fn key_value_iter_from_key<K: AsRef<[u8]>>(&self, key: K) -> MerkleKeyValueStream<'_, NodeStore<T, S>> {
//         MerkleKeyValueStream::from(self)
//     }
// }
// // pub struct KeyValueIterator<'db, T> {
// //     stream: Option<MerkleKeyValueStream<'db, T>>,
// //     merkle: T,
// // }
// //
// // impl<'a, T: TrieReader> KeyValueIterator<'a, T> {
// //     pub fn new(nodestore: T) -> Self {
// //         let mut it = KeyValueIterator {
// //             stream: None,
// //             merkle: Arc::new(nodestore),
// //         };
// //         it
// //     }
// //
// //     pub fn next(&mut self){
// //         self.stream = Some(MerkleKeyValueStream::from(self.merkle.clone()));
// //     }
// // }