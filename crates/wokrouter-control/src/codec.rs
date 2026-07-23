use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::ControlError;

pub(crate) const MAX_FRAME_BYTES: u32 = 1024 * 1024;

pub(crate) async fn read_frame<T, R>(reader: &mut R) -> Result<T, ControlError>
where
    T: DeserializeOwned,
    R: AsyncRead + Unpin,
{
    let mut prefix = [0_u8; size_of::<u32>()];
    reader.read_exact(&mut prefix).await?;
    let length = u32::from_be_bytes(prefix);
    if length > MAX_FRAME_BYTES {
        return Err(ControlError::FrameTooLarge {
            length,
            max: MAX_FRAME_BYTES,
        });
    }

    let mut body = vec![0_u8; length as usize];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map_err(|error| ControlError::InvalidFrame {
        message: error.to_string(),
    })
}

pub(crate) async fn write_frame<T, W>(writer: &mut W, frame: &T) -> Result<(), ControlError>
where
    T: Serialize,
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(frame).map_err(|error| ControlError::InvalidFrame {
        message: error.to_string(),
    })?;
    let length = u32::try_from(body.len()).map_err(|_| ControlError::FrameTooLarge {
        length: u32::MAX,
        max: MAX_FRAME_BYTES,
    })?;
    if length > MAX_FRAME_BYTES {
        return Err(ControlError::FrameTooLarge {
            length,
            max: MAX_FRAME_BYTES,
        });
    }

    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}
