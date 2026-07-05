// internal crates
use miru_agent::cache::errors::{ReceiveActorMessageErr, SendActorMessageErr};
use miru_agent::errors::Error;
use miru_agent::scan::UploadErr;

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

mod from_conversions {
    use super::*;

    #[test]
    fn send_actor_message_err_maps() {
        let err: UploadErr = send_actor_msg_err().into();
        assert!(matches!(err, UploadErr::SendActorMessageErr(_)));
    }

    #[test]
    fn receive_actor_message_err_maps() {
        let err: UploadErr = recv_actor_msg_err().into();
        assert!(matches!(err, UploadErr::ReceiveActorMessageErr(_)));
    }
}

mod error_trait {
    use super::*;

    // Exercise the impl_error! arms (code/http_status/is_network_conn_err/params)
    // for both UploadErr variants.
    #[test]
    fn delegates_error_trait_methods() {
        let send: UploadErr = send_actor_msg_err().into();
        let recv: UploadErr = recv_actor_msg_err().into();
        for err in [send, recv] {
            let _ = err.code();
            let _ = err.http_status();
            assert!(!err.is_network_conn_err());
            let _ = err.params();
        }
    }
}
