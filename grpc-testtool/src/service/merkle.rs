use std::sync::Arc;

use crate::merkle::{
    merkle_server::Merkle as MerkleServiceTrait, IteratorErrorRequest, IteratorNextRequest,
    IteratorNextResponse, IteratorReleaseRequest, NewProposalRequest, NewProposalResponse,
    NewViewRequest, NewViewResponse, ProposalCommitRequest, PutRequest, ViewGetRequest,
    ViewGetResponse, ViewHasRequest, ViewHasResponse, ViewNewIteratorWithStartAndPrefixRequest,
    ViewNewIteratorWithStartAndPrefixResponse, ViewReleaseRequest,
};
use firewood::{
    db::{BatchOp, Proposal},
    v2::api::{self, Db},
};
use tonic::{async_trait, Request, Response, Status};

use super::{IntoStatusResultExt, View};

use futures::StreamExt;

#[async_trait]
impl MerkleServiceTrait for super::Database {
    async fn new_proposal(
        &self,
        req: Request<NewProposalRequest>,
    ) -> Result<Response<NewProposalResponse>, Status> {
        let req = req.into_inner();

        // convert the provided data into a set of BatchOp Put and Delete operations
        let mut data: Vec<_> = req
            .puts
            .into_iter()
            .map(|put| BatchOp::Put {
                key: put.key,
                value: put.value,
            })
            .collect();
        data.extend(
            req.deletes
                .into_iter()
                .map(|del| BatchOp::Delete { key: del }),
        );

        let mut views = self.views.lock().await;

        // the proposal depends on the parent_view_id. If it's None, then we propose on the db itself
        // Otherwise, we're provided a base proposal to base it off of, so go fetch that
        // proposal from the views
        let proposal = match req.parent_id {
            None => self.db.propose(data).await,
            Some(parent_id) => {
                let view = views.map.get(&parent_id);
                match view {
                    None => return Err(Status::not_found(format!("ID {parent_id} not found"))),
                    Some(View::Proposal(parent)) => {
                        firewood::v2::api::Proposal::propose(parent.clone(), data).await
                    }
                    Some(_) => {
                        return Err(Status::invalid_argument(format!(
                            "ID {parent_id} is not a commitable proposal"
                        )))
                    }
                }
            }
        }
        .into_status_result()?;

        // compute the next view id
        let id = views.insert(View::Proposal(Arc::new(proposal)));

        let resp = Response::new(NewProposalResponse { id });
        Ok(resp)
    }

    async fn proposal_commit(
        &self,
        req: Request<ProposalCommitRequest>,
    ) -> Result<Response<()>, Status> {
        let mut views = self.views.lock().await;
        let id = req.into_inner().id;

        match views.map.remove(&id) {
            None => return Err(Status::not_found(format!("id {id} not found"))),
            Some(View::Proposal(proposal)) => proposal.commit(),
            Some(_) => {
                return Err(Status::invalid_argument(format!(
                    "id {id} is not a commitable proposal"
                )))
            }
        }
        .await
        .into_status_result()?;
        Ok(Response::new(()))
    }

    async fn new_view(
        &self,
        req: Request<NewViewRequest>,
    ) -> Result<Response<NewViewResponse>, Status> {
        let hash = std::convert::TryInto::<[u8; 32]>::try_into(req.into_inner().root_hash)
            .map_err(|_| api::Error::InvalidProposal) // TODO: better error here?
            .into_status_result()?;
        let mut views = self.views.lock().await;
        let view = self.db.revision(hash).await.into_status_result()?;
        let id = views.insert(View::Historical(view));
        Ok(Response::new(NewViewResponse { id }))
    }

    async fn view_has(
        &self,
        _req: Request<ViewHasRequest>,
    ) -> Result<Response<ViewHasResponse>, Status> {
        todo!()
    }

    async fn view_get(
        &self,
        _req: Request<ViewGetRequest>,
    ) -> Result<Response<ViewGetResponse>, Status> {
        todo!()
    }

    async fn view_new_iterator_with_start_and_prefix(
        &self,
        req: Request<ViewNewIteratorWithStartAndPrefixRequest>,
    ) -> Result<Response<ViewNewIteratorWithStartAndPrefixResponse>, Status> {
        let req = req.into_inner();
        let id = req.id;
        let key = req.start;
        let views = self
            .views
            .lock()
            .await;
        let iter = views
            .get(id as u32)
            .ok_or_else(|| Status::not_found("id {id} not found"));
        let iter = iter
            .map(|view| match view {
                View::Historical(historical) => historical.stream_from(key.into_boxed_slice()),
                View::Proposal(_proposal) => todo!(), // proposal.stream_from(key),
            })?;
        let mut iterators = self.iterators.lock().await;
        let id = iterators.insert(Box::pin(iter));
        Ok(Response::new(ViewNewIteratorWithStartAndPrefixResponse { id }))
    }

    async fn view_release(&self, req: Request<ViewReleaseRequest>) -> Result<Response<()>, Status> {
        let mut views = self.views.lock().await;
        // we don't care if this works :/
        views.delete(req.into_inner().id);
        Ok(Response::new(()))
    }

    async fn iterator_next(
        &self,
        req: Request<IteratorNextRequest>,
    ) -> Result<Response<IteratorNextResponse>, Status> {
        let id = req.into_inner().id;
        let mut iterators = self.iterators.lock().await;
        let view = iterators
            .get_mut(id)
            .ok_or_else(|| Status::not_found(format!("iterator {id} not found")))?;

        let (key, value) = view
            .next()
            .await
            .ok_or_else(|| Status::out_of_range(format!("iterator {id} at end")))?
            .into_status_result()?;
        Ok(Response::new(IteratorNextResponse {
            data: Some(PutRequest { key: key.to_vec(), value }),
        }))
    }

    async fn iterator_release(
        &self,
        _req: Request<IteratorReleaseRequest>,
    ) -> Result<Response<()>, Status> {
        todo!()
    }

    async fn iterator_error(
        &self,
        _req: Request<IteratorErrorRequest>,
    ) -> Result<Response<()>, Status> {
        todo!()
    }
}
