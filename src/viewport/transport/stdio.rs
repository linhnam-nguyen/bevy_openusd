//! JSON Lines transport for a native parent process.
//!
//! Reading stdin and writing stdout are deliberately isolated in background
//! threads. Bevy systems only use non-blocking channel operations, so a slow
//! host or an idle stdin pipe cannot stall rendering, input, or Frost.

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

use bevy::prelude::*;
use viewport_protocol::{
    ViewportCommandEnvelope, ViewportEventEnvelope, ViewportWireMessage, decode_json_line,
    encode_json_line,
};

use crate::viewport::api::{ViewportBridgeSet, ViewportCommandInbox, ViewportEventOutbox};

/// Upper bounds keep a busy external client from monopolizing one Bevy frame.
const MAX_COMMANDS_PER_FRAME: usize = 256;
const MAX_EVENTS_PER_FRAME: usize = 512;

/// Installs native stdin/stdout workers around the existing viewport bridge.
pub(crate) struct StdioTransportPlugin;

impl Plugin for StdioTransportPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(StdioTransport::spawn())
            .add_systems(
                Update,
                drain_stdin_commands.before(ViewportBridgeSet::ApplyCommands),
            )
            .add_systems(
                Update,
                drain_viewport_events.after(ViewportBridgeSet::PublishStageLoadState),
            );
    }
}

/// Process-local channels connecting blocking standard streams to the ECS.
///
/// `Receiver` is wrapped in a mutex because Bevy resources must be `Sync`.
/// The update system only locks it long enough to drain immediately available
/// messages; it never waits for a host process.
#[derive(Resource)]
struct StdioTransport {
    inbound_commands: Mutex<Receiver<ViewportCommandEnvelope>>,
    outbound_events: Sender<ViewportEventEnvelope>,
    stdout_open: Arc<AtomicBool>,
}

impl StdioTransport {
    fn spawn() -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let stdout_open = Arc::new(AtomicBool::new(true));

        if let Err(error) = thread::Builder::new()
            .name("viewport-stdio-reader".to_owned())
            .spawn(move || read_stdin_commands(command_sender))
        {
            // Diagnostic output must stay on stderr: stdout is reserved for
            // protocol records whenever this transport is enabled.
            eprintln!("[usdview stdio] failed to start stdin reader: {error}");
        }

        let worker_stdout_open = Arc::clone(&stdout_open);
        if let Err(error) = thread::Builder::new()
            .name("viewport-stdio-writer".to_owned())
            .spawn(move || write_stdout_events(event_receiver, worker_stdout_open))
        {
            stdout_open.store(false, Ordering::Release);
            eprintln!("[usdview stdio] failed to start stdout writer: {error}");
        }

        Self {
            inbound_commands: Mutex::new(command_receiver),
            outbound_events: event_sender,
            stdout_open,
        }
    }
}

/// Transfers already-buffered external commands into the existing bridge.
fn drain_stdin_commands(transport: Res<StdioTransport>, mut inbox: ResMut<ViewportCommandInbox>) {
    let Ok(receiver) = transport.inbound_commands.lock() else {
        eprintln!("[usdview stdio] stdin command queue lock was poisoned");
        return;
    };

    for _ in 0..MAX_COMMANDS_PER_FRAME {
        match receiver.try_recv() {
            Ok(envelope) => inbox.push(envelope),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
}

/// Transfers bridge events to the stdout writer without performing pipe I/O in
/// an ECS system. The unbounded channel preserves events while its dedicated
/// writer is blocked by a temporarily slow parent process.
fn drain_viewport_events(mut outbox: ResMut<ViewportEventOutbox>, transport: Res<StdioTransport>) {
    for _ in 0..MAX_EVENTS_PER_FRAME {
        let Some(event) = outbox.pop() else {
            break;
        };

        if !transport.stdout_open.load(Ordering::Acquire) {
            continue;
        }

        if transport.outbound_events.send(event).is_err()
            && transport.stdout_open.swap(false, Ordering::AcqRel)
        {
            eprintln!("[usdview stdio] stdout writer disconnected; protocol events are dropped");
        }
    }
}

/// Blocks on standard input in a dedicated worker and forwards only commands.
fn read_stdin_commands(sender: Sender<ViewportCommandEnvelope>) {
    let stdin = io::stdin();
    receive_json_lines(BufReader::new(stdin.lock()), sender);
}

fn receive_json_lines<R: BufRead>(reader: R, sender: Sender<ViewportCommandEnvelope>) {
    for (line_number, line) in reader.lines().enumerate() {
        let line_number = line_number + 1;
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("[usdview stdio] stdin read failed: {error}");
                return;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        match decode_json_line(&line) {
            Ok(ViewportWireMessage::Command(envelope)) => {
                if sender.send(envelope).is_err() {
                    return;
                }
            }
            Ok(ViewportWireMessage::Event(_)) => {
                eprintln!(
                    "[usdview stdio] ignored event received on stdin at line {line_number}; stdin accepts commands only"
                );
            }
            Err(error) => {
                eprintln!("[usdview stdio] invalid JSON at stdin line {line_number}: {error}");
            }
        }
    }
}

/// Blocks on the event channel in a dedicated worker and writes only JSON
/// Lines event records to stdout. Bevy's default log layer writes to stderr,
/// and this module never prints diagnostics to stdout.
fn write_stdout_events(receiver: Receiver<ViewportEventEnvelope>, stdout_open: Arc<AtomicBool>) {
    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    if let Err(error) = write_event_lines(receiver, &mut writer, Arc::clone(&stdout_open)) {
        stdout_open.store(false, Ordering::Release);
        eprintln!("[usdview stdio] stdout write failed: {error}");
    }
}

fn write_event_lines<W: Write>(
    receiver: Receiver<ViewportEventEnvelope>,
    writer: &mut W,
    stdout_open: Arc<AtomicBool>,
) -> io::Result<()> {
    while let Ok(event) = receiver.recv() {
        let line = match encode_json_line(&ViewportWireMessage::Event(event)) {
            Ok(line) => line,
            Err(error) => {
                eprintln!("[usdview stdio] could not serialize viewport event: {error}");
                continue;
            }
        };

        writer.write_all(line.as_bytes())?;
        writer.flush()?;
    }

    stdout_open.store(false, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use viewport_protocol::{ViewportCommand, ViewportEvent};

    #[test]
    fn reader_forwards_versioned_commands_without_waiting_for_ecs() {
        let input = encode_json_line(&ViewportWireMessage::Command(ViewportCommandEnvelope::new(
            "native-1",
            ViewportCommand::RequestSnapshot,
        )))
        .unwrap();
        let (sender, receiver) = mpsc::channel();

        receive_json_lines(Cursor::new(input), sender);

        let command = receiver.try_recv().expect("reader should forward command");
        assert_eq!(command.request_id, "native-1");
        assert!(matches!(command.command, ViewportCommand::RequestSnapshot));
    }

    #[test]
    fn writer_emits_one_event_record_per_envelope() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(ViewportEventEnvelope::new(
                Some("native-2".to_owned()),
                ViewportEvent::Ready {
                    protocol_version: viewport_protocol::PROTOCOL_VERSION,
                },
            ))
            .unwrap();
        drop(sender);

        let stdout_open = Arc::new(AtomicBool::new(true));
        let mut output = Vec::new();
        write_event_lines(receiver, &mut output, Arc::clone(&stdout_open)).unwrap();

        let line = String::from_utf8(output).unwrap();
        assert_eq!(line.matches('\n').count(), 1);
        assert!(matches!(
            decode_json_line(&line).unwrap(),
            ViewportWireMessage::Event(ViewportEventEnvelope {
                event: ViewportEvent::Ready { .. },
                ..
            })
        ));
        assert!(!stdout_open.load(Ordering::Acquire));
    }

    #[test]
    fn startup_ready_event_reaches_the_worker_channel_on_the_first_update() {
        let (command_sender, command_receiver) = mpsc::channel();
        drop(command_sender);
        let (event_sender, event_receiver) = mpsc::channel();
        let stdout_open = Arc::new(AtomicBool::new(true));
        let mut app = App::new();
        app.insert_resource(StdioTransport {
            inbound_commands: Mutex::new(command_receiver),
            outbound_events: event_sender,
            stdout_open,
        })
        .init_resource::<ViewportEventOutbox>()
        .add_systems(Startup, emit_ready_for_test)
        .add_systems(
            Update,
            drain_viewport_events.after(ViewportBridgeSet::PublishStageLoadState),
        );

        app.update();

        assert!(matches!(
            event_receiver
                .try_recv()
                .expect("Ready should be forwarded"),
            ViewportEventEnvelope {
                event: ViewportEvent::Ready { .. },
                ..
            }
        ));
    }

    fn emit_ready_for_test(mut outbox: ResMut<ViewportEventOutbox>) {
        outbox.push(ViewportEventEnvelope::new(
            None,
            ViewportEvent::Ready {
                protocol_version: viewport_protocol::PROTOCOL_VERSION,
            },
        ));
    }
}
