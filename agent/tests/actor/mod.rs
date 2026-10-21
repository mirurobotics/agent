// standard crates
use std::future;

// internal crates
use miru_agent::actor::{self, Actor, ActorErr, Flow, Mailbox};
use miru_agent::errors::{Error, HTTPCode};

// external crates
use tokio::sync::{mpsc, oneshot};

enum Command {
    Echo {
        value: u32,
        respond_to: oneshot::Sender<u32>,
    },
    Stop {
        respond_to: oneshot::Sender<()>,
    },
}

/// Echoes values back and counts what it handled.
struct Echo {
    commands: mpsc::Receiver<Command>,
    handled: u32,
}

impl Actor for Echo {
    type Command = Command;

    fn commands(&mut self) -> &mut mpsc::Receiver<Command> {
        &mut self.commands
    }

    async fn handle(&mut self, cmd: Command) -> Flow {
        self.handled += 1;
        match cmd {
            Command::Echo { value, respond_to } => {
                actor::respond("echo", "echo", respond_to, value);
                Flow::Continue
            }
            Command::Stop { respond_to } => {
                actor::respond("echo", "stop", respond_to, ());
                Flow::Shutdown
            }
        }
    }
}

fn spawn_echo() -> (Mailbox<Command>, Echo) {
    let (mailbox, commands) = actor::channel("echo", 8);
    (
        mailbox,
        Echo {
            commands,
            handled: 0,
        },
    )
}

async fn echo(mailbox: &Mailbox<Command>, value: u32) -> Result<u32, ActorErr> {
    mailbox
        .request("echo", |respond_to| Command::Echo { value, respond_to })
        .await
}

async fn stop(mailbox: &Mailbox<Command>) -> Result<(), ActorErr> {
    mailbox
        .request("stop", |respond_to| Command::Stop { respond_to })
        .await
}

mod serve_next {
    use super::*;

    #[tokio::test]
    async fn handles_one_command() {
        let (mailbox, mut echo_actor) = spawn_echo();

        let (answer, flow) = tokio::join!(echo(&mailbox, 7), actor::serve_next(&mut echo_actor));

        assert_eq!(7, answer.unwrap());
        assert_eq!(Flow::Continue, flow);
        assert_eq!(1, echo_actor.handled);
    }

    #[tokio::test]
    async fn passes_on_a_shutdown() {
        let (mailbox, mut echo_actor) = spawn_echo();

        let (answer, flow) = tokio::join!(stop(&mailbox), actor::serve_next(&mut echo_actor));

        answer.unwrap();
        assert_eq!(Flow::Shutdown, flow);
    }

    #[tokio::test]
    async fn every_mailbox_dropped_is_a_shutdown() {
        let (mailbox, mut echo_actor) = spawn_echo();
        drop(mailbox);

        assert_eq!(Flow::Shutdown, actor::serve_next(&mut echo_actor).await);
        assert_eq!(0, echo_actor.handled);
    }
}

mod serve_until {
    use super::*;

    #[tokio::test]
    async fn returns_the_result() {
        let (_mailbox, mut echo_actor) = spawn_echo();

        let result = actor::serve_until(&mut echo_actor, future::ready(3)).await;

        assert_eq!(Some(3), result);
    }

    #[tokio::test]
    async fn answers_commands_while_the_future_runs() {
        let (mailbox, mut echo_actor) = spawn_echo();
        let (open, gate) = oneshot::channel::<u32>();
        // the gate opens only after the actor has answered, so the answer
        // must come while serve_until is still waiting on the gate
        let caller = async {
            let answers = (echo(&mailbox, 1).await, echo(&mailbox, 2).await);
            open.send(5).unwrap();
            answers
        };

        let (answers, result) = tokio::join!(caller, actor::serve_until(&mut echo_actor, gate));

        assert_eq!((1, 2), (answers.0.unwrap(), answers.1.unwrap()));
        assert_eq!(Some(Ok(5)), result);
        assert_eq!(2, echo_actor.handled);
    }

    #[tokio::test]
    async fn a_shutdown_drops_the_future() {
        let (mailbox, mut echo_actor) = spawn_echo();

        let (answer, result) = tokio::join!(
            stop(&mailbox),
            actor::serve_until(&mut echo_actor, future::pending::<()>())
        );

        answer.unwrap();
        assert_eq!(None, result);
    }

    #[tokio::test]
    async fn every_mailbox_dropped_drops_the_future() {
        let (mailbox, mut echo_actor) = spawn_echo();
        drop(mailbox);

        let result = actor::serve_until(&mut echo_actor, future::pending::<()>()).await;

        assert_eq!(None, result);
    }
}

mod request {
    use super::*;

    #[tokio::test]
    async fn a_stopped_actor_is_a_send_error() {
        let (mailbox, echo_actor) = spawn_echo();
        drop(echo_actor);

        let err = echo(&mailbox, 1).await.unwrap_err();

        assert!(matches!(err, ActorErr::SendActorMessageErr(_)), "{err:?}");
        assert!(err.to_string().contains("failed to send actor message"));
    }

    #[tokio::test]
    async fn a_command_dropped_unanswered_is_a_receive_error() {
        let (mailbox, mut echo_actor) = spawn_echo();
        let drop_one = async {
            drop(echo_actor.commands.recv().await);
        };

        let (result, ()) = tokio::join!(echo(&mailbox, 1), drop_one);

        let err = result.unwrap_err();
        assert!(
            matches!(err, ActorErr::ReceiveActorMessageErr(_)),
            "{err:?}"
        );
        assert!(err.to_string().contains("failed to receive actor message"));
    }

    #[tokio::test]
    async fn errors_are_internal() {
        let (mailbox, echo_actor) = spawn_echo();
        drop(echo_actor);
        let send_err = echo(&mailbox, 1).await.unwrap_err();
        let (mailbox, mut echo_actor) = spawn_echo();
        let (receive_err, ()) = tokio::join!(echo(&mailbox, 1), async {
            drop(echo_actor.commands.recv().await);
        });

        for err in [send_err, receive_err.unwrap_err()] {
            assert_eq!("internal_server_error", err.code().as_str());
            assert_eq!(HTTPCode::INTERNAL_SERVER_ERROR, err.http_status());
            assert!(!err.is_network_conn_err());
            assert!(!err.is_terminal());
            assert_eq!(None, err.params());
        }
    }
}

mod respond {
    use super::*;

    #[test]
    fn a_caller_that_stopped_waiting_is_not_an_error() {
        let (respond_to, response) = oneshot::channel::<u32>();
        drop(response);

        actor::respond("echo", "echo", respond_to, 1);
    }
}

#[test]
fn mailbox_debug_names_its_label() {
    let (mailbox, _commands) = actor::channel::<Command>("echo", 1);

    assert!(format!("{mailbox:?}").contains("\"echo\""));
}
