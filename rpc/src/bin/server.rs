use rpc::{rpcdb::database_server::DatabaseServer, service::database::DatabaseService};

use tonic::transport::Server;

// TODO: use clap to parse command line input to run the server
#[tokio::main]
async fn main() {
    let addr = "[::1]:10000".parse().unwrap();

    println!("Database-Server listening on: {}", addr);

    let svc = DatabaseService::default();
    let svc = DatabaseServer::new(svc);

    // TODO: graceful shutdown
    Server::builder()
        .add_service(svc)
        .serve(addr)
        .await
        .unwrap();
}
