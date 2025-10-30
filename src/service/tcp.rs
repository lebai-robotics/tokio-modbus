// SPDX-FileCopyrightText: Copyright (c) 2017-2023 slowtec GmbH <post@slowtec.de>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    fmt,
    io::{Error, ErrorKind},
    pin::Pin,
    sync::atomic::{AtomicU16, Ordering},
    task::{Context, Poll},
};

use futures_util::task::noop_waker_ref;
use futures_util::{sink::SinkExt as _, stream::StreamExt as _};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::codec::Framed;

use crate::{
    codec,
    frame::{tcp::*, *},
    slave::*,
};

const INITIAL_TRANSACTION_ID: TransactionId = 0;

/// Modbus TCP client
#[derive(Debug)]
pub(crate) struct Client<T> {
    framed: Framed<T, codec::tcp::ClientCodec>,
    unit_id: UnitId,
    transaction_id: AtomicU16,
}

impl<T> Client<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub(crate) fn new(transport: T, slave: Slave) -> Self {
        let framed = Framed::new(transport, codec::tcp::ClientCodec::default());
        let unit_id: UnitId = slave.into();
        let transaction_id = AtomicU16::new(INITIAL_TRANSACTION_ID);
        Self {
            framed,
            unit_id,
            transaction_id,
        }
    }

    fn next_transaction_id(&self) -> TransactionId {
        let transaction_id = self.transaction_id.load(Ordering::Relaxed);
        self.transaction_id
            .store(transaction_id.wrapping_add(1), Ordering::Relaxed);
        transaction_id
    }

    fn next_request_hdr(&self, unit_id: UnitId) -> Header {
        let transaction_id = self.next_transaction_id();
        Header {
            transaction_id,
            unit_id,
        }
    }

    fn next_request_adu<R>(&self, req: R, disconnect: bool) -> RequestAdu
    where
        R: Into<RequestPdu>,
    {
        RequestAdu {
            hdr: self.next_request_hdr(self.unit_id),
            pdu: req.into(),
            disconnect,
        }
    }

    fn clear_old_data(&mut self) {
        let tx = self.framed.write_buffer_mut();
        if !tx.is_empty() {
            log::info!("clear tx data: {:02X?}", tx);
            tx.clear();
        }
        let rx = self.framed.read_buffer_mut();
        if !rx.is_empty() {
            log::info!("clear rx data: {:02X?}", rx);
            rx.clear();
        }

        let mut buf_old = [0; 4096];
        let mut data_old = ReadBuf::new(&mut buf_old);
        if let Poll::Ready(Err(e)) = Pin::new(self.framed.get_mut())
            .poll_read(&mut Context::from_waker(noop_waker_ref()), &mut data_old)
        {
            log::warn!("recv old data err: {:?}", e);
        }
        let data_old = data_old.filled();
        if !data_old.is_empty() {
            log::info!("clear old data: {:02X?}", data_old);
        }
    }

    pub(crate) async fn call(&mut self, req: Request) -> Result<Response, Error> {
        log::debug!("Call {:?}", req);
        let disconnect = req == Request::Disconnect;
        let req_adu = self.next_request_adu(req, disconnect);
        let req_hdr = req_adu.hdr;

        self.clear_old_data();
        self.framed.send(req_adu).await?;
        let res_adu = self
            .framed
            .next()
            .await
            .ok_or_else(Error::last_os_error)??;

        match res_adu.pdu {
            ResponsePdu(Ok(res)) => verify_response_header(req_hdr, res_adu.hdr).and(Ok(res)),
            ResponsePdu(Err(err)) => Err(Error::new(ErrorKind::Other, err)),
        }
    }
}

fn verify_response_header(req_hdr: Header, rsp_hdr: Header) -> Result<(), Error> {
    if req_hdr != rsp_hdr {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "Invalid response header: expected/request = {req_hdr:?}, actual/response = {rsp_hdr:?}"
            ),
        ));
    }
    Ok(())
}

impl<T> SlaveContext for Client<T> {
    fn set_slave(&mut self, slave: Slave) {
        self.unit_id = slave.into();
    }
}

#[async_trait::async_trait]
impl<T> crate::client::Client for Client<T>
where
    T: fmt::Debug + AsyncRead + AsyncWrite + Send + Unpin,
{
    async fn call(&mut self, req: Request) -> Result<Response, Error> {
        Client::call(self, req).await
    }
}
