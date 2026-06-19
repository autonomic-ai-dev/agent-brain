use std::net::SocketAddr;
use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::engine::Engine;
use crate::grpc::convert::{route_request_from_proto, route_response_to_proto};
use crate::grpc::pb::routing_service_server::{RoutingService, RoutingServiceServer};
use crate::grpc::pb::{HealthRequest, HealthResponse, RouteTaskRequest, RouteTaskResponse};

pub struct RoutingGrpc {
    engine: Arc<Engine>,
}

impl RoutingGrpc {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl RoutingService for RoutingGrpc {
    async fn route_task(
        &self,
        request: Request<RouteTaskRequest>,
    ) -> Result<Response<RouteTaskResponse>, Status> {
        let req = route_request_from_proto(request.into_inner());
        let cwd = req.cwd.as_deref().map(std::path::Path::new);
        let resp = self
            .engine
            .route_task(
                &req.user_message,
                cwd,
                &req.open_files,
                req.max_tokens,
                req.limits,
                req.phase.as_deref(),
                req.task_kind.as_deref(),
            )
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(route_response_to_proto(resp)))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            version: env!("CARGO_PKG_VERSION").to_string(),
            ready: true,
        }))
    }
}

pub async fn serve(engine: Arc<Engine>, addr: SocketAddr) -> anyhow::Result<()> {
    let svc = RoutingGrpc::new(engine);
    tracing::info!(%addr, "agent-brain gRPC listening");
    tonic::transport::Server::builder()
        .add_service(RoutingServiceServer::new(svc))
        .serve(addr)
        .await?;
    Ok(())
}
