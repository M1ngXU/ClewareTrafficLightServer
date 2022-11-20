use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::Duration;

use cleware_traffic_light::{Color, InitializedGlobalDevice, State};

fn main() {
    let handler = Handler::create();
    let incoming =
        TcpListener::bind(SocketAddr::new(local_ip_address::local_ip().unwrap(), 6633)).unwrap();
    println!("Listening on: {}", incoming.local_addr().unwrap());
    while let Ok((mut stream, _)) = incoming.accept() {
        if let Some((color, state)) = handle_connection(&mut stream) {
            let _ = stream.write_all("HTTP/1.1 200 OK\r\n\r\n".as_bytes());
            handler.send(color, state);
        } else {
            let _ = stream.write_all("HTTP/1.1 400 BAD REQUEST\r\n\r\n".as_bytes());
        }
        let _ = stream.shutdown(Shutdown::Both);
    }
}

fn handle_connection(stream: &mut TcpStream) -> Option<(Color, ExtendedState)> {
    let mut b = [0; 100];
    let len = stream.read(&mut b).ok()?;
    let s = String::from_utf8_lossy(&b[..len]);
    let mut path = s
        .split_once("/set/")?
        .1
        .split_once(" HTTP/1.1\r\n")?
        .0
        .split('/');
    let parsed_color = match path.next()?.to_lowercase().as_str() {
        "red" => Color::Red,
        "yellow" => Color::Yellow,
        "green" => Color::Green,
        _ => return None,
    };
    let parsed_state = match path.next()?.to_lowercase().as_str() {
        "off" => ExtendedState::Off,
        "on" => ExtendedState::On,
        "blinking" => {
            let off = Duration::from_millis(path.next()?.parse().ok()?);
            let on = Duration::from_millis(path.next()?.parse().ok()?);
            ExtendedState::Blinking { off, on }
        }
        _ => return None,
    };
    Some((parsed_color, parsed_state))
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ExtendedState {
    Off,
    On,
    Blinking { off: Duration, on: Duration },
}

struct Handler {
    sender: Sender<(Color, ExtendedState)>,
}
impl Handler {
    fn create() -> Self {
        let (sender, receiver) = mpsc::channel();
        let light = Arc::new(InitializedGlobalDevice::create_with_any().unwrap());
        std::thread::spawn(move || {
            for color in [Color::Red, Color::Yellow, Color::Green] {
                light.set_light(color, State::Off).unwrap();
            }
            let (thread_killer_red_s, thread_killer_red_r) = mpsc::channel();
            let (thread_killer_yellow_s, thread_killer_yellow_r) = mpsc::channel();
            let (thread_killer_green_s, thread_killer_green_r) = mpsc::channel();
            for (color, killer) in [
                (Color::Red, thread_killer_red_r),
                (Color::Yellow, thread_killer_yellow_r),
                (Color::Green, thread_killer_green_r),
            ] {
                let light = light.clone();
                std::thread::spawn(move || {
                    'outer: for (off, on) in killer.iter().flatten() {
                        loop {
                            if let Ok(None) = killer.recv_timeout(off) {
                                continue 'outer;
                            }
                            light.set_light(color, State::On).unwrap();
                            if let Ok(None) = killer.recv_timeout(on) {
                                continue 'outer;
                            }
                            light.set_light(color, State::Off).unwrap();
                        }
                    }
                });
            }
            let thread_killer_s = [
                thread_killer_red_s,
                thread_killer_yellow_s,
                thread_killer_green_s,
            ];
            for (color, state) in receiver.into_iter() {
                let color_usize = (color as u8 % 0x10) as usize;
                thread_killer_s[color_usize].send(None).unwrap();
                match state {
                    ExtendedState::Off => light.set_light(color, State::Off).unwrap(),
                    ExtendedState::On => light.set_light(color, State::On).unwrap(),
                    ExtendedState::Blinking { off, on } => {
                        thread_killer_s[color_usize].send(Some((off, on))).unwrap();
                    }
                }
            }
        });
        Self { sender }
    }

    fn send(&self, color: Color, state: ExtendedState) {
        self.sender.send((color, state)).unwrap();
    }
}
