pub mod convert;
pub mod server;

pub mod pb {
    tonic::include_proto!("agent_brain.v1");
}

pub use server::serve;

#[cfg(test)]
mod tests {
    use super::convert::{route_response_to_proto, task_kind_to_proto};
    use agent_brain::types::{RouteTaskResponse, TaskKind};

    #[test]
    fn grpc_route_response_includes_bridge_fields() {
        let mut resp = RouteTaskResponse {
            task_kind: Some("verification".into()),
            route_confidence: 0.82,
            escalate_recommended: false,
            ..Default::default()
        };
        let pb = route_response_to_proto(resp.clone());
        assert_eq!(pb.route_confidence, 0.82);
        assert_eq!(pb.task_kind, task_kind_to_proto(TaskKind::Verification));
        resp.route_confidence = 0.1;
        resp.escalate_recommended = true;
        let pb = route_response_to_proto(resp);
        assert!(pb.escalate_recommended);
    }
}
