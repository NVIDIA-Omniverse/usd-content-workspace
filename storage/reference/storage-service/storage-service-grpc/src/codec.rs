// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::marker::PhantomData;

use bytes::{BufMut as _, Bytes};
use tonic::{
    codec::{Codec, EncodeBuf, Encoder},
    Status,
};
use tonic_prost::{prost::Message, ProstDecoder};

/// Allows sending pre-encoded request messages and parsing reply messages with the default decoder.
#[derive(Default, Clone, Copy)]
pub struct EncodedRequestCodec<T> {
    _phantom: PhantomData<T>,
}

impl<T: Message + Default + 'static> Codec for EncodedRequestCodec<T> {
    type Encode = Bytes;
    type Decode = T;
    type Encoder = EncodedRequestEncoder;
    type Decoder = ProstDecoder<T>;

    fn encoder(&mut self) -> Self::Encoder {
        EncodedRequestEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        Default::default()
    }
}

pub struct EncodedRequestEncoder;

impl Encoder for EncodedRequestEncoder {
    type Item = Bytes;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        dst.put_slice(&item);
        Ok(())
    }
}
