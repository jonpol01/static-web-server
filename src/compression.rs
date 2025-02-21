// SPDX-License-Identifier: MIT OR Apache-2.0
// This file is part of Static Web Server.
// See https://static-web-server.net/ for more information
// Copyright (C) 2019-present Jose Quintana <joseluisq.net>

//! Auto-compression module to compress responses body.
//!

// Part of the file is borrowed from <https://github.com/seanmonstar/warp/pull/513>*

#[cfg(feature = "compression-brotli")]
use async_compression::tokio::bufread::BrotliEncoder;
#[cfg(feature = "compression-deflate")]
use async_compression::tokio::bufread::DeflateEncoder;
#[cfg(feature = "compression-gzip")]
use async_compression::tokio::bufread::GzipEncoder;
#[cfg(feature = "compression-zstd")]
use async_compression::tokio::bufread::ZstdEncoder;

use bytes::Bytes;
use futures_util::Stream;
use headers::{AcceptEncoding, ContentCoding, ContentType, HeaderMap, HeaderMapExt, HeaderValue};
use hyper::{
    header::{CONTENT_ENCODING, CONTENT_LENGTH},
    Body, Method, Response,
};
use mime_guess::Mime;
use pin_project::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_util::io::{ReaderStream, StreamReader};

use crate::{exts::http::MethodExt, Result};

/// Contains a fixed list of common text-based MIME types in order to apply compression.
pub const TEXT_MIME_TYPES: [&str; 24] = [
    "text/html",
    "text/css",
    "text/javascript",
    "text/xml",
    "text/plain",
    "text/csv",
    "text/calendar",
    "text/markdown",
    "text/x-yaml",
    "text/x-toml",
    "text/x-component",
    "application/rtf",
    "application/xhtml+xml",
    "application/javascript",
    "application/x-javascript",
    "application/json",
    "application/xml",
    "application/rss+xml",
    "application/atom+xml",
    "font/truetype",
    "font/opentype",
    "application/vnd.ms-fontobject",
    "image/svg+xml",
    "application/wasm",
];

/// Create a wrapping handler that compresses the Body of a [`hyper::Response`]
/// using `gzip`, `deflate`, `brotli` or `zstd` if is specified in the `Accept-Encoding` header, adding
/// `content-encoding: <coding>` to the Response's [`HeaderMap`].
/// It also provides the ability to apply compression for text-based MIME types only.
pub fn auto(
    method: &Method,
    headers: &HeaderMap<HeaderValue>,
    resp: Response<Body>,
) -> Result<Response<Body>> {
    // Skip compression for HEAD and OPTIONS request methods
    if method.is_head() || method.is_options() {
        return Ok(resp);
    }

    // Compress response based on Accept-Encoding header
    if let Some(encoding) = get_prefered_encoding(headers) {
        // Skip compression for non-text-based MIME types
        if let Some(content_type) = resp.headers().typed_get::<ContentType>() {
            let mime = Mime::from(content_type);
            if !TEXT_MIME_TYPES.iter().any(|h| *h == mime) {
                return Ok(resp);
            }
        }

        #[cfg(feature = "compression-gzip")]
        if encoding == ContentCoding::GZIP {
            let (head, body) = resp.into_parts();
            return Ok(gzip(head, body.into()));
        }

        #[cfg(feature = "compression-deflate")]
        if encoding == ContentCoding::DEFLATE {
            let (head, body) = resp.into_parts();
            return Ok(deflate(head, body.into()));
        }

        #[cfg(feature = "compression-brotli")]
        if encoding == ContentCoding::BROTLI {
            let (head, body) = resp.into_parts();
            return Ok(brotli(head, body.into()));
        }

        #[cfg(feature = "compression-zstd")]
        if encoding == ContentCoding::ZSTD {
            let (head, body) = resp.into_parts();
            return Ok(zstd(head, body.into()));
        }
    }

    Ok(resp)
}

/// Create a wrapping handler that compresses the Body of a [`Response`].
/// using gzip, adding `content-encoding: gzip` to the Response's [`HeaderMap`].
#[cfg(feature = "compression-gzip")]
#[cfg_attr(docsrs, doc(cfg(feature = "compression-gzip")))]
pub fn gzip(
    mut head: http::response::Parts,
    body: CompressableBody<Body, hyper::Error>,
) -> Response<Body> {
    tracing::trace!("compressing response body on the fly using gzip");

    let body = Body::wrap_stream(ReaderStream::new(GzipEncoder::new(StreamReader::new(body))));
    let header = create_encoding_header(head.headers.remove(CONTENT_ENCODING), ContentCoding::GZIP);
    head.headers.remove(CONTENT_LENGTH);
    head.headers.append(CONTENT_ENCODING, header);
    Response::from_parts(head, body)
}

/// Create a wrapping handler that compresses the Body of a [`Response`].
/// using deflate, adding `content-encoding: deflate` to the Response's [`HeaderMap`].
#[cfg(feature = "compression-deflate")]
#[cfg_attr(docsrs, doc(cfg(feature = "compression-deflate")))]
pub fn deflate(
    mut head: http::response::Parts,
    body: CompressableBody<Body, hyper::Error>,
) -> Response<Body> {
    tracing::trace!("compressing response body on the fly using deflate");

    let body = Body::wrap_stream(ReaderStream::new(DeflateEncoder::new(StreamReader::new(
        body,
    ))));
    let header = create_encoding_header(
        head.headers.remove(CONTENT_ENCODING),
        ContentCoding::DEFLATE,
    );
    head.headers.remove(CONTENT_LENGTH);
    head.headers.append(CONTENT_ENCODING, header);
    Response::from_parts(head, body)
}

/// Create a wrapping handler that compresses the Body of a [`Response`].
/// using brotli, adding `content-encoding: br` to the Response's [`HeaderMap`].
#[cfg(feature = "compression-brotli")]
#[cfg_attr(docsrs, doc(cfg(feature = "compression-brotli")))]
pub fn brotli(
    mut head: http::response::Parts,
    body: CompressableBody<Body, hyper::Error>,
) -> Response<Body> {
    tracing::trace!("compressing response body on the fly using brotli");

    let body = Body::wrap_stream(ReaderStream::new(BrotliEncoder::new(StreamReader::new(
        body,
    ))));
    let header =
        create_encoding_header(head.headers.remove(CONTENT_ENCODING), ContentCoding::BROTLI);
    head.headers.remove(CONTENT_LENGTH);
    head.headers.append(CONTENT_ENCODING, header);
    Response::from_parts(head, body)
}

/// Create a wrapping handler that compresses the Body of a [`Response`].
/// using zstd, adding `content-encoding: zstd` to the Response's [`HeaderMap`].
#[cfg(feature = "compression-zstd")]
#[cfg_attr(docsrs, doc(cfg(feature = "compression-zstd")))]
pub fn zstd(
    mut head: http::response::Parts,
    body: CompressableBody<Body, hyper::Error>,
) -> Response<Body> {
    tracing::trace!("compressing response body on the fly using zstd");

    let body = Body::wrap_stream(ReaderStream::new(ZstdEncoder::new(StreamReader::new(body))));
    let header = create_encoding_header(head.headers.remove(CONTENT_ENCODING), ContentCoding::ZSTD);
    head.headers.remove(CONTENT_LENGTH);
    head.headers.append(CONTENT_ENCODING, header);
    Response::from_parts(head, body)
}

/// Given an optional existing encoding header, appends to the existing or creates a new one.
pub fn create_encoding_header(existing: Option<HeaderValue>, coding: ContentCoding) -> HeaderValue {
    if let Some(val) = existing {
        if let Ok(str_val) = val.to_str() {
            return HeaderValue::from_str(&[str_val, ", ", coding.to_static()].concat())
                .unwrap_or_else(|_| coding.into());
        }
    }
    coding.into()
}

/// Try to get the prefered `content-encoding` via the `accept-encoding` header.
pub fn get_prefered_encoding(headers: &HeaderMap<HeaderValue>) -> Option<ContentCoding> {
    if let Some(ref accept_encoding) = headers.typed_get::<AcceptEncoding>() {
        return accept_encoding.prefered_encoding();
    }
    None
}

/// A wrapper around any type that implements [`Stream`](futures_util::Stream) to be
/// compatible with async_compression's `Stream` based encoders.
#[pin_project]
#[derive(Debug)]
pub struct CompressableBody<S, E>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: std::error::Error,
{
    #[pin]
    body: S,
}

impl<S, E> Stream for CompressableBody<S, E>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: std::error::Error,
{
    type Item = std::io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        use std::io::{Error, ErrorKind};

        let pin = self.project();
        S::poll_next(pin.body, ctx).map_err(|_| Error::from(ErrorKind::InvalidData))
    }
}

impl From<Body> for CompressableBody<Body, hyper::Error> {
    fn from(body: Body) -> Self {
        CompressableBody { body }
    }
}
