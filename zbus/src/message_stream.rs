use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use async_broadcast::Receiver as ActiveReceiver;
use futures_core::{ready, stream};
use futures_util::stream::FusedStream;
use ordered_stream::{OrderedStream, PollResult};
use static_assertions::assert_impl_all;

use crate::{Connection, ConnectionInner, Message, MessageSequence, Result};

/// A [`stream::Stream`] implementation that yields [`Message`] items.
///
/// You can convert a [`Connection`] to this type and back to [`Connection`].
///
/// **NOTE**: You must ensure a `MessageStream` is continuously polled or you will experience hangs.
/// If you don't need to continuously poll the `MessageStream` but need to keep it around for later
/// use, keep the connection around and convert it into a `MessageStream` when needed. The
/// conversion is not an expensive operation so you don't need to  worry about performance, unless
/// you do it very frequently. If you need to convert back and forth frequently, you may want to
/// consider keeping both a connection and stream around.
#[derive(Clone, Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct MessageStream {
    conn_inner: Arc<ConnectionInner>,
    msg_receiver: ActiveReceiver<Result<Arc<Message>>>,
    last_seq: MessageSequence,
}

assert_impl_all!(MessageStream: Send, Sync, Unpin);

impl stream::Stream for MessageStream {
    type Item = Result<Arc<Message>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        Pin::new(&mut this.msg_receiver).poll_next(cx).map(|msg| {
            if let Some(Ok(msg)) = &msg {
                this.last_seq = msg.recv_position();
            }

            msg
        })
    }
}

impl OrderedStream for MessageStream {
    type Data = Result<Arc<Message>>;
    type Ordering = MessageSequence;

    fn poll_next_before(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        before: Option<&Self::Ordering>,
    ) -> Poll<PollResult<Self::Ordering, Self::Data>> {
        let this = self.get_mut();
        if let Some(before) = before {
            if this.last_seq >= *before {
                return Poll::Ready(PollResult::NoneBefore);
            }
        }
        if let Some(msg) = ready!(stream::Stream::poll_next(Pin::new(this), cx)) {
            Poll::Ready(PollResult::Item {
                data: msg,
                ordering: this.last_seq,
            })
        } else {
            Poll::Ready(PollResult::Terminated)
        }
    }
}

impl FusedStream for MessageStream {
    fn is_terminated(&self) -> bool {
        self.msg_receiver.is_terminated()
    }
}

impl From<Connection> for MessageStream {
    fn from(conn: Connection) -> Self {
        let conn_inner = conn.inner.clone();
        let msg_receiver = conn_inner.msg_receiver.activate_cloned();

        Self {
            conn_inner,
            msg_receiver,
            last_seq: Default::default(),
        }
    }
}

impl From<&Connection> for MessageStream {
    fn from(conn: &Connection) -> Self {
        Self::from(conn.clone())
    }
}

impl From<MessageStream> for Connection {
    fn from(stream: MessageStream) -> Connection {
        Connection::from(&stream)
    }
}

impl From<&MessageStream> for Connection {
    fn from(stream: &MessageStream) -> Connection {
        Connection {
            inner: stream.conn_inner.clone(),
        }
    }
}
