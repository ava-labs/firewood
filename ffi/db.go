// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import "context"

// DB is the common interface for interacting with a Firewood database.
// It is implemented by [LocalDB] (backed by the FFI [Database]) and by
// the remote gRPC adapter in the remote package.
type DB interface {
	Get(ctx context.Context, key []byte) ([]byte, error)
	Update(ctx context.Context, batch []BatchOp) (Hash, error)
	Propose(ctx context.Context, batch []BatchOp) (DBProposal, error)
	Root() Hash
	Close(ctx context.Context) error
}

// DBProposal is the common interface for an uncommitted proposal.
type DBProposal interface {
	Root() Hash
	Commit(ctx context.Context) error
	Drop() error
	Get(ctx context.Context, key []byte) ([]byte, error)
	Iter(ctx context.Context, key []byte) (DBIterator, error)
	Propose(ctx context.Context, batch []BatchOp) (DBProposal, error)
}

// DBIterator is the common interface for iterating key-value pairs.
type DBIterator interface {
	Next() bool
	Key() []byte
	Value() []byte
	Err() error
	Drop() error
}

// Compile-time interface checks.
var (
	_ DB         = (*LocalDB)(nil)
	_ DBProposal = (*localProposal)(nil)
	_ DBIterator = (*localIterator)(nil)
)

// LocalDB wraps a [Database] to satisfy the [DB] interface.
type LocalDB struct {
	db *Database
}

// NewLocalDB returns a [DB] backed by the given FFI [Database].
func NewLocalDB(db *Database) DB {
	return &LocalDB{db: db}
}

func (l *LocalDB) Get(_ context.Context, key []byte) ([]byte, error) {
	return l.db.Get(key)
}

func (l *LocalDB) Update(_ context.Context, batch []BatchOp) (Hash, error) {
	return l.db.Update(batch)
}

func (l *LocalDB) Propose(_ context.Context, batch []BatchOp) (DBProposal, error) {
	p, err := l.db.Propose(batch)
	if err != nil {
		return nil, err
	}
	return &localProposal{p: p}, nil
}

func (l *LocalDB) Root() Hash {
	return l.db.Root()
}

func (l *LocalDB) Close(ctx context.Context) error {
	return l.db.Close(ctx)
}

// localProposal wraps a [Proposal] to satisfy the [DBProposal] interface.
type localProposal struct {
	p *Proposal
}

func (lp *localProposal) Root() Hash {
	return lp.p.Root()
}

func (lp *localProposal) Commit(_ context.Context) error {
	return lp.p.Commit()
}

func (lp *localProposal) Drop() error {
	return lp.p.Drop()
}

func (lp *localProposal) Get(_ context.Context, key []byte) ([]byte, error) {
	return lp.p.Get(key)
}

func (lp *localProposal) Iter(_ context.Context, key []byte) (DBIterator, error) {
	it, err := lp.p.Iter(key)
	if err != nil {
		return nil, err
	}
	return &localIterator{it: it}, nil
}

func (lp *localProposal) Propose(_ context.Context, batch []BatchOp) (DBProposal, error) {
	p, err := lp.p.Propose(batch)
	if err != nil {
		return nil, err
	}
	return &localProposal{p: p}, nil
}

// localIterator wraps an [Iterator] to satisfy the [DBIterator] interface.
type localIterator struct {
	it *Iterator
}

func (li *localIterator) Next() bool {
	return li.it.Next()
}

func (li *localIterator) Key() []byte {
	return li.it.Key()
}

func (li *localIterator) Value() []byte {
	return li.it.Value()
}

func (li *localIterator) Err() error {
	return li.it.Err()
}

func (li *localIterator) Drop() error {
	return li.it.Drop()
}
