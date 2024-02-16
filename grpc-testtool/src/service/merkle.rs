use crate::merkle::{
    merkle_server::Merkle as MerkleServiceTrait, NewProposalRequest, NewProposalResponse,
    NewViewRequest, NewViewResponse, ProposalCommitRequest, ProposalCommitResponse, ViewGetRequest,
    ViewGetResponse, ViewHasRequest, ViewHasResponse, ViewNewIteratorWithStartAndPrefixRequest,
    ViewNewIteratorWithStartAndPrefixResponse, ViewReleaseRequest,
};
use tonic::{async_trait, Request, Response, Status};

#[async_trait]
impl MerkleServiceTrait for super::Db {
    async fn new_proposal(
        &self,
        _req: Request<NewProposalRequest>,
    ) -> Result<Response<NewProposalResponse>, Status> {
        todo!()
    }

    async fn proposal_commit(
        &self,
        _req: Request<ProposalCommitRequest>,
    ) -> Result<Response<ProposalCommitResponse>, Status> {
        todo!()
    }

    async fn new_view(
        &self,
        _req: Request<NewViewRequest>,
    ) -> Result<Response<NewViewResponse>, Status> {
        todo!()
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
