use tokio::{io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter}, net::TcpListener, select};
use std::{collections::HashMap, net::SocketAddr, sync::{Arc, Mutex}};
use colored::*;

type Db = Arc<Mutex<HashMap<String, SocketAddr>>>;

#[tokio::main]
async fn main()
{
    println!("Server started at: {}", std::env::args().nth(1).unwrap());
    let listner = TcpListener::bind(std::env::args().nth(1).unwrap()).await.unwrap();
    let db : Db = Arc::new(Mutex::new(HashMap::new()));
    let (message_sender, receiver) = tokio::sync::broadcast::channel::<(String, SocketAddr)>(32);
    drop(receiver);

    loop {
        let (socket, client_socket_address) = listner.accept().await.unwrap();
        let db = db.clone();
        let mut client_message_receiver = message_sender.subscribe();
        let client_message_sender = message_sender.clone();
        tokio::spawn(async move {
            handle_client(socket, client_socket_address, db, client_message_receiver, client_message_sender).await;
        });
    }
}

async fn handle_client(mut socket: tokio::net::TcpStream,
                    client_socket_addr: SocketAddr,
                    db: Db,
                    mut client_message_receiver: tokio::sync::broadcast::Receiver<(String, SocketAddr)>, 
                    client_message_sender: tokio::sync::broadcast::Sender<(String, SocketAddr)>
                )
{
    // send list of usernames
    let usernames = {
        let db = db.lock().unwrap();
        db.keys().cloned().collect::<Vec<String>>()
    };

    println!("Existing Users: {:?}", usernames);

    socket.write_all("Existing Users: ".as_bytes()).await.unwrap();
    socket.write_all(usernames.join(", ").as_bytes()).await.unwrap();
    socket.write_all(b"\n").await.unwrap();
    socket.write_all(b"Enter your username: ").await.unwrap();

    let mut buffer = [0u8; 1024];
    let mut username = String::new();

    loop {
        let n = socket.read(&mut buffer as &mut [u8]).await.unwrap();
        let input = String::from_utf8_lossy(&buffer[..n]);
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        username = input.to_string();
        println!("Username: {}", username);
        let mut contain = {
            let db = db.lock().unwrap();
            db.contains_key(&username)
        };
        if contain {
            socket.write_all(b"Username already exists. Please enter a new username: ").await.unwrap();
        } else {
            {
                let mut db = db.lock().unwrap();
                db.insert(username.clone(), client_socket_addr);
            }
            socket.write_all(b"Username accepted.\n ---------- Welcome to Chat ----------\n\n").await.unwrap();
            break;
        }
    }

    // start chatting 
    let (stream_reader, stream_writer) = socket.split();
    let mut stream_buf_writer: BufWriter<tokio::net::tcp::WriteHalf<'_>> = BufWriter::new(stream_writer);
    let mut stream_buf_reader: BufReader<tokio::net::tcp::ReadHalf<'_>> = BufReader::new(stream_reader);
    let mut client_input = String::new();

    loop 
    {
        select! {
            _ = stream_buf_reader.read_line(&mut client_input) => {
                let msg = format!("{}: {} \n", username.green(), client_input.trim());
                _ = client_message_sender.send((msg.trim().to_string(), client_socket_addr));
                client_input.clear();
            },
            Ok((message, message_client_socket_addr)) = client_message_receiver.recv() => {
                if message_client_socket_addr != client_socket_addr {
                    let msg = format!("{}\n", message);
                    stream_buf_writer.write_all(msg.as_bytes()).await.unwrap();
                    stream_buf_writer.flush().await.unwrap();
                }
            },
        }
    }
}