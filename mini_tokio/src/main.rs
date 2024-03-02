use futures::future::BoxFuture;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};
// A utility that allows us to implement a `std::task::Waker` without having to
// use `unsafe` code.
use futures::task::{self, ArcWake};
// Used as a channel to queue scheduled tasks.
use crossbeam::channel;

thread_local! {
    static CURRENT: RefCell<Option<channel::Sender<Arc<Task>>>> =
        RefCell::new(None);
}

pub fn spawn<F>(future: F)
    where
    F: Future<Output = ()> + Send + 'static
{
    CURRENT.with(|cell| {
        let borrow = cell.borrow();
        let sender = borrow.as_ref().unwrap();
        Task::spawn(sender,future);
    });
}

// Declaring mini-tokio struct
struct MiniTokio {
    // We use sender and reciever queues to schedule tasks
    scheduled: channel::Receiver<Arc<Task>>,    // reciever side that runs the tasks
    sender: channel::Sender<Arc<Task>>          // tasks are scheduled here
}

// impl for MiniTokio
impl MiniTokio {
    // constructor for MiniTokio
    fn new() -> MiniTokio {
        let (sender, reciever) = channel::unbounded();
        MiniTokio {
            scheduled: reciever,
            sender: sender
        }
    }

    // spawn a future
    fn spawn<F>(&self, future: F) 
        where 
        F: Future<Output = ()> + Send + 'static
    {
        Task::spawn(&self.sender,future);
    }

    // run
    fn run(&self)
    {
        CURRENT.with(|cell| {
            *cell.borrow_mut() = Some(self.sender.clone());
        });

        while let Ok(task) = self.scheduled.recv() {
            task.poll();
        }
    }
}

// Represents async tasks
struct Task {
    // wrapped in mutex to make Task 'sync'
    // Only one thread attempts to execute the future
    future: Mutex<BoxFuture<'static, ()>>,

    // when notified the Task in scheduled queued in this channel
    // poped on the other side for execution by Executor
    executor: channel::Sender<Arc<Task>>
}

impl Task {
    fn spawn<F>(sender: &channel::Sender<Arc<Task>>, future: F)
        where
        F: Future<Output = ()> + Send + 'static
    {
        let new_task = Arc::new(Task {
            future: Mutex::new(Box::pin(future)),
            executor: sender.clone()
        });

        let _ = sender.send(new_task);
    }

    fn poll(self: Arc<Task>)
    {
        // get waker and context for the task
        let waker = task::waker(self.clone());
        let mut cx = Context::from_waker(&waker);

        let mut future = self.future.try_lock().unwrap();
        let _ = future.as_mut().poll(&mut cx);
    }
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>)
    {
        let _ = arc_self.executor.send(arc_self.clone());
    }
}

// delay
async fn delay(dur: Duration)
{
    struct Delay {
        when: Instant,
        waker: Option<Arc<Mutex<Waker>>>
    }

    impl Future for Delay {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            // if already preset => stored waker = current waker
            if let Some(waker) = &self.waker {
                let mut waker = waker.lock().unwrap();

                if !waker.will_wake(cx.waker()) {
                    *waker = cx.waker().clone();
                }
            } else {
                let when  = self.when;
                let waker = Arc::new(Mutex::new(cx.waker().clone()));
                self.waker = Some(waker.clone());

                thread::spawn(move || {
                    let now = Instant::now();
                    if now < when {
                        thread::sleep(when-now);
                    }

                    let waker = waker.lock().unwrap();
                    waker.wake_by_ref();
                });
            }

            if Instant::now() >= self.when {
                Poll::Ready(())
            } else {
                Poll::Pending
            }

        }
    }

    // Create instance of Delay
    let future = Delay {
        when: Instant::now() + dur,
        waker: None,
    };

    // wait for duration to complete
    future.await;

}

fn main() {
    let mini_tokio = MiniTokio::new();

    mini_tokio.spawn(async {
        spawn(async {
            println!("world");
        });
        
        spawn(async {
            delay(Duration::from_millis(1000)).await;
            println!("hello");
        });

        delay(Duration::from_millis(2000)).await;
        std::process::exit(0);
    });

    mini_tokio.run();
}
