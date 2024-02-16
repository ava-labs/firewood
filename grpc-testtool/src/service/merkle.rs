use std::sync::Arc;

use crate::merkle::{
    merkle_server::Merkle as MerkleServiceTrait, NewProposalRequest, NewProposalResponse,
    NewViewRequest, NewViewResponse, ProposalCommitRequest, ProposalCommitResponse, ViewGetRequest,
    ViewGetResponse, ViewHasRequest, ViewHasResponse, ViewNewIteratorWithStartAndPrefixRequest,
    ViewNewIteratorWithStartAndPrefixResponse, ViewReleaseRequest,
};
use firewood::{
    db::{BatchOp, Proposal},
    v2::api::{self, Db},
};
use tonic::{async_trait, Request, Response, Status};

use super::{IntoStatusResultExt, View};

//#[prost(uint32, optional, tag = "1")]
//pub parent_view_id: ::core::option::Option<u32>,
//#[prost(message, repeated, tag = "2")]
//pub puts: ::prost::alloc::vec::Vec<PutRequest>,
//#[prost(bytes = "vec", repeated, tag = "3")]
//pub deletes: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,

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
        let proposal = match req.parent_view_id {
            None => self.db.propose(data).await,
            Some(parent_id) => {
                let view = views.map.get(&parent_id);
                match view {
                    None => return Err(Status::invalid_argument("invalid view id")),
                    Some(View::Proposal(parent)) => {
                        firewood::v2::api::Proposal::propose(parent.clone(), data).await
                    }
                    Some(_) => return Err(Status::invalid_argument("non-proposal id")),
                }
            }
        }
        .into_status_result()?;

        // compute the next view id
        let view_id = views.insert(View::Proposal(Arc::new(proposal)));

        let resp = Response::new(NewProposalResponse {
            proposal_id: view_id,
        });
        Ok(resp)
    }

    async fn proposal_commit(
        &self,
        req: Request<ProposalCommitRequest>,
    ) -> Result<Response<ProposalCommitResponse>, Status> {
        let mut views = self.views.lock().await;

        match views.map.remove(&req.into_inner().proposal_id) {
            None => return Err(Status::invalid_argument("invalid view id")),
            Some(View::Proposal(proposal)) => proposal.commit(),
            Some(_) => return Err(Status::invalid_argument("non-proposal id")),
        }
        .await
        .into_status_result()?;
        Ok(Response::new(ProposalCommitResponse { err: 0 }))
    }

    async fn new_view(
        &self,
        req: Request<NewViewRequest>,
    ) -> Result<Response<NewViewResponse>, Status> {
        let hash = std::convert::TryInto::<[u8; 32]>::try_into(req.into_inner().root_id)
            .map_err(|_| api::Error::InvalidProposal) // TODO: better error here?
            .into_status_result()?;
        let mut views = self.views.lock().await;
        let view = self.db.revision(hash).await.into_status_result()?;
        let view_id = views.insert(View::Historical(view));
        Ok(Response::new(NewViewResponse { view_id }))
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
        _req: Request<ViewNewIteratorWithStartAndPrefixRequest>,
    ) -> Result<Response<ViewNewIteratorWithStartAndPrefixResponse>, Status> {
        todo!()
    }

    async fn view_release(
        &self,
        _req: Request<ViewReleaseRequest>,
    ) -> Result<Response<()>, Status> {
        todo!()
    }
}
