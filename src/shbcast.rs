//! # //! Single broadcast channel for all subscribed clients
//!
//! **Author**: "Dany LE"
//!
use latpr::tunnel::{CallbackEvent, IOInterest, Msg, MsgKind, Topic};
use latpr::utils::*;
use latpr::utils::{LogLevel, LOG};
use latpr::{ERROR, EXIT, INFO, WARN};
use std::collections::HashMap;
use std::env;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::panic;
use std::process::{Child, Command, Stdio};
use std::string::String;

fn step_handle(
    evt: &CallbackEvent,
    clients: &mut HashMap<u16, String>,
    topic: &mut Topic,
    process: &mut Child,
) -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if let Some(msg) = evt.msg {
        match msg.kind {
            MsgKind::ChannelSubscribe => {
                let user = String::from(std::str::from_utf8(&msg.data[0..msg.size as usize - 1])?);
                clients.insert(msg.client_id, user);
                INFO!("Client {} subscribe to channel {}", msg.client_id, &args[2]);
            }
            MsgKind::ChannelUnsubscribe => {
                WARN!(
                    "Client {} unsubscribe to channel {}",
                    msg.client_id,
                    &args[2]
                );
                if let None = clients.remove(&msg.client_id) {
                    WARN!("Client {} is not in the client list", msg.client_id);
                }
            }
            MsgKind::ChannelUnsubscribeAll => {
                INFO!("Unsubcribed all clients from channel {}", args[2]);
                for (key, _) in clients.iter_mut() {
                    let msg = Msg::create(MsgKind::ChannelUnsubscribe, 0, *key, Vec::new());
                    topic.write(&msg)?;
                }
                clients.clear();
            }
            MsgKind::ChannelData => {
                // write data to child
                if let Some(mut stdin) = process.stdin.as_ref() {
                    stdin.write_all(&msg.data)?;
                }
            }
            _ => {
                WARN!(
                    "Receive mesage kind {} from client {}",
                    msg.kind,
                    msg.client_id
                );
            }
        };
    }
    let event = match evt.event {
        None => return Ok(()),
        Some(e) => e,
    };
    let _ = match evt.fd {
        None => return Ok(()),
        Some(d) => d,
    };
    if event.is_readable() {
        // got data send it to client
        let mut buf = [0; 2048];
        if let Some(stdout) = process.stdout.as_mut() {
            let n = stdout.read(&mut buf[..])?;
            INFO!("Sending {} bytes of raw data to all clients", n);
            for (key, _) in clients.iter() {
                let msg = Msg::create(MsgKind::ChannelData, 0, *key, (&buf[0..n]).to_vec());
                topic.write(&msg)?;
            }
        }
    }
    Ok(())
}

fn clean_up(n: i32) {
    if n != 0 {
        panic!(
            "{}",
            format!("Service is terminated by system signal: {}", n)
        );
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // init the system log
    // Create an empty log object and keep it alive in the scope
    // of `main`. When this object is dropped, the syslog will
    // be closed automatically
    let _log = LOG::init_log();
    on_exit(clean_up);
    // read all the arguments
    let args: Vec<String> = env::args().collect();
    // there must be minimum 4 arguments:
    // - the program
    // - the socket file
    // - the topic name
    // - the command to run
    if args.len() != 4 {
        EXIT!("Invalid arguments: {}", format!("{:?}", args));
    }
    let mut clients = HashMap::<u16, String>::new();
    //init the process
    let mut process = Command::new(&args[3])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let fd = process
        .stdout
        .as_ref()
        .ok_or("Unable to get child process STDOUT")?
        .as_raw_fd();
    let mut msg_handle = |evt: &CallbackEvent, topic: &mut Topic| {
        step_handle(evt, &mut clients, topic, &mut process)
    };
    {
        let mut topic = Topic::create(&args[2], &args[1]);
        let mut running = true;
        topic.on_message(&mut msg_handle);
        // init the broadcast process
        topic.register_io(fd, IOInterest::READABLE)?;
        topic.open()?;
        while running {
            if let Err(error) = topic.step() {
                ERROR!("Error step: {}", error);
                running = false;
            }
        }
    }
    Ok(())
}
