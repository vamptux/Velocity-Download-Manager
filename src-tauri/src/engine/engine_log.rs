use crate::model::{DownloadLogEntry, DownloadLogLevel, DownloadRecord};

use super::helpers::unix_epoch_millis;

const ENGINE_LOG_LIMIT: usize = 64;

pub(super) fn append_download_log(
    download: &mut DownloadRecord,
    level: DownloadLogLevel,
    code: impl Into<String>,
    message: impl Into<String>,
) {
    let code = code.into();
    let message = message.into();
    let timestamp = unix_epoch_millis();

    if let Some(last) = download.engine_log.last_mut()
        && last.level == level
        && last.code == code
        && last.message == message
    {
        last.timestamp = timestamp;
        return;
    }

    download.engine_log.push(DownloadLogEntry {
        timestamp,
        level,
        code,
        message,
    });
    if download.engine_log.len() > ENGINE_LOG_LIMIT {
        let overflow = download.engine_log.len().saturating_sub(ENGINE_LOG_LIMIT);
        download.engine_log.drain(0..overflow);
    }
}
