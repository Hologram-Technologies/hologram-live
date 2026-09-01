wit_bindgen::generate!({
    path: "../../../specs/wit/channel-subscribe",
    world: "application",
    generate_all,
});

struct ChannelSubscribe;

impl exports::hologram::application_channel_subscribe::guest::Guest for ChannelSubscribe {
    fn run(
        input: Vec<u8>,
    ) -> Result<Vec<u8>, exports::hologram::application_channel_subscribe::guest::GuestError> {
        let channel = String::from_utf8(input).map_err(|_| {
            exports::hologram::application_channel_subscribe::guest::GuestError {
                code: exports::hologram::application_channel_subscribe::guest::ErrorCode::InvalidInput,
                message: "channel must be UTF-8".to_owned(),
            }
        })?;
        hologram::host::channel_subscribe::try_receive(&channel)
            .map(|message| message.unwrap_or_default())
            .map_err(|message| {
                exports::hologram::application_channel_subscribe::guest::GuestError {
                    code: exports::hologram::application_channel_subscribe::guest::ErrorCode::Failed,
                    message,
                }
            })
    }
}

export!(ChannelSubscribe);
