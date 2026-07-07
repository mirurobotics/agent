// internal crates
use miru_agent::cache::errors::{ReceiveActorMessageErr, SendActorMessageErr};
use miru_agent::errors::Error;
use miru_agent::scan::errors::{DuplicateCollectionID, InternalError, InvalidRule};
use miru_agent::scan::ScanErr;

fn send_actor_msg_err() -> SendActorMessageErr {
    SendActorMessageErr {
        source: Box::new(std::io::Error::other("send failed")),
        trace: miru_agent::trace!(),
    }
}

fn recv_actor_msg_err() -> ReceiveActorMessageErr {
    ReceiveActorMessageErr {
        source: Box::new(std::io::Error::other("recv failed")),
        trace: miru_agent::trace!(),
    }
}

fn invalid_rule() -> InvalidRule {
    InvalidRule {
        existing_upload_collection_id: "coll-old".to_string(),
        replacement_upload_collection_id: "coll-new".to_string(),
        trace: miru_agent::trace!(),
    }
}

fn duplicate_collection_id() -> DuplicateCollectionID {
    DuplicateCollectionID {
        collection_id: "coll-dup".to_string(),
        trace: miru_agent::trace!(),
    }
}

fn internal_error() -> InternalError {
    InternalError {
        message: "boom".to_string(),
        trace: miru_agent::trace!(),
    }
}

mod from_conversions {
    use super::*;

    #[test]
    fn send_actor_message_err_maps() {
        let err: ScanErr = send_actor_msg_err().into();
        assert!(matches!(err, ScanErr::SendActorMessageErr(_)));
    }

    #[test]
    fn receive_actor_message_err_maps() {
        let err: ScanErr = recv_actor_msg_err().into();
        assert!(matches!(err, ScanErr::ReceiveActorMessageErr(_)));
    }
}

mod variant_construction {
    use super::*;

    // InvalidRule is only ever built inside the collection scanner's set_config
    // guard, which is unreachable through the actor's collection-id-keyed
    // update_rules. Construct + wrap it directly here to cover the arm and its
    // Display/error-trait plumbing.
    #[test]
    fn invalid_rule_wraps_and_displays() {
        let err = ScanErr::InvalidRule(invalid_rule());
        assert!(matches!(err, ScanErr::InvalidRule(_)));
        let msg = err.to_string();
        assert!(msg.contains("coll-old"));
        assert!(msg.contains("coll-new"));
    }

    #[test]
    fn duplicate_collection_id_wraps_and_displays() {
        let err = ScanErr::DuplicateCollectionID(duplicate_collection_id());
        assert!(matches!(err, ScanErr::DuplicateCollectionID(_)));
        assert!(err.to_string().contains("coll-dup"));
    }

    #[test]
    fn internal_error_wraps_and_displays() {
        let err = ScanErr::InternalError(internal_error());
        assert!(matches!(err, ScanErr::InternalError(_)));
        assert!(err.to_string().contains("boom"));
    }
}

mod error_trait {
    use super::*;

    // Exercise the impl_error! arms (code/http_status/is_network_conn_err/params)
    // for every ScanErr variant reachable here.
    #[test]
    fn delegates_error_trait_methods() {
        let errs: [ScanErr; 5] = [
            send_actor_msg_err().into(),
            recv_actor_msg_err().into(),
            ScanErr::InvalidRule(invalid_rule()),
            ScanErr::DuplicateCollectionID(duplicate_collection_id()),
            ScanErr::InternalError(internal_error()),
        ];
        for err in errs {
            let _ = err.code();
            let _ = err.http_status();
            assert!(!err.is_network_conn_err());
            let _ = err.params();
        }
    }
}
