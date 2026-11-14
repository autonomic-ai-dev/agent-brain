use std::net::SocketAddr;
use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::grpc::convert::{route_request_from_proto, route_response_to_proto};
use crate::grpc::pb::routing_service_server::{RoutingService, RoutingServiceServer};
use crate::grpc::pb::{HealthRequest, HealthResponse, RouteTaskRequest, RouteTaskResponse};
use tekmor::engine::{Engine, RouteRequest};

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
            .route_task(RouteRequest {
                user_message: &req.user_message,
                cwd,
                open_files: &req.open_files,
                max_tokens: req.max_tokens,
                limits: req.limits,
                explicit_phase: req.phase.as_deref(),
                explicit_task_kind: req.task_kind.as_deref(),
            })
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
    tracing::info!(%addr, "tekmor gRPC listening");
    tonic::transport::Server::builder()
        .add_service(RoutingServiceServer::new(svc))
        .serve(addr)
        .await?;
    Ok(())
}
