use std::{
    thread,
    sync::{
        Arc,
    },
    time,
    io::ErrorKind
};
use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;

use super::{
    Connection,
    SocketConnection,
};
use models::Message;
use error::{Result, Error};


type Tx = Sender<Message>;
type Rx = Receiver<Message>;

#[derive(Clone)]
pub struct Manager {
    connection: Arc<Option<Mutex<SocketConnection>>>,
    client_id: u64,
    outbound: (Rx, Tx),
    inbound: (Rx, Tx),
    handshake_completed: bool,
}

impl Manager {
    pub fn new(client_id: u64) -> Self {
        let connection = Arc::new(None);
        let (sender_o, receiver_o) = unbounded();
        let (sender_i, receiver_i) = unbounded();

        Self {
            connection,
            client_id,
            handshake_completed: false,
            inbound: (receiver_i, sender_i),
            outbound: (receiver_o, sender_o),
        }
    }

    pub fn start(self) {
        let manager_inner = self.clone();
        thread::spawn(move || {
            send_and_receive_loop(manager_inner);
        });
    }

    pub fn send(&self, message: Message) -> Result<()> {
        self.outbound.1.send(message).unwrap();
        Ok(())
    }

    pub fn recv(&self) -> Result<Message> {
        let message = self.inbound.0.recv().unwrap();
        Ok(message)
    }

    fn connect(&mut self) -> Result<()> {
        if self.connection.is_some() {
            return Ok(());
        }

        debug!("Connecting");

        let mut new_connection = SocketConnection::connect()?;

        debug!("Performing handshake");
        new_connection.handshake(self.client_id)?;
        debug!("Handshake completed");

        self.connection = Arc::new(Some(Mutex::new(new_connection)));

        debug!("Connected");

        Ok(())
    }

    fn disconnect(&mut self) {
        self.handshake_completed = false;
        self.connection = Arc::new(None);
    }

}


fn send_and_receive_loop(mut manager: Manager) {
    debug!("Starting sender loop");

    let mut inbound = manager.inbound.1.clone();
    let outbound = manager.outbound.0.clone();

    loop {
        let connection = manager.connection.clone();

        match *connection {
            Some(ref conn) => {
                let mut connection = conn.lock();
                match send_and_receive(&mut *connection, &mut inbound, &outbound) {
                    Err(Error::IoError(ref err)) if err.kind() == ErrorKind::WouldBlock => (),
                    Err(Error::IoError(_)) | Err(Error::ConnectionClosed) => manager.disconnect(),
                    Err(why) => error!("error: {}", why),
                    _ => (),
                }

                thread::sleep(time::Duration::from_millis(500));
            },
            None => {
                match manager.connect() {
                    Err(err) => {
                        match err {
                            Error::IoError(ref err) if err.kind() == ErrorKind::ConnectionRefused => (),
                            why => error!("Failed to connect: {:?}", why),
                        }
                        thread::sleep(time::Duration::from_secs(10));
                    },
                    _ => manager.handshake_completed = true,
                }
            }
        }
    }
}

fn send_and_receive(connection: &mut SocketConnection, inbound: &mut Tx, outbound: &Rx) -> Result<()> {
    while let Ok(msg) = outbound.try_recv() {
        connection.send(msg).expect("Failed to send outgoing data");
    }

    let msg = connection.recv()?;
    inbound.send(msg).expect("Failed to send received data");

    Ok(())
}
