wit_bindgen::generate!({
    path: "../../../specs/wit/channel-publish",
    world: "application",
    generate_all,
});

struct ChannelPublish;

impl exports::hologram::application_channel_publish::guest::Guest for ChannelPublish {
    fn run(
        input: Vec<u8>,
    ) -> Result<Vec<u8>, exports::hologram::application_channel_publish::guest::GuestError> {
        let Some(separator) = input.iter().position(|byte| *byte == b'\n') else {
            return Err(invalid_input("expected channel, newline, and message"));
        };
        let channel = String::from_utf8(input[..separator].to_vec())
            .map_err(|_| invalid_input("channel must be UTF-8"))?;
        hologram::host::channel_publish::publish(&channel, &input[separator + 1..]).map_err(
            |message| exports::hologram::application_channel_publish::guest::GuestError {
                code: exports::hologram::application_channel_publish::guest::ErrorCode::Failed,
                message,
            },
        )?;
        Ok(channel.into_bytes())
    }
}

fn invalid_input(
    message: &str,
) -> exports::hologram::application_channel_publish::guest::GuestError {
    exports::hologram::application_channel_publish::guest::GuestError {
        code: exports::hologram::application_channel_publish::guest::ErrorCode::InvalidInput,
        message: message.to_owned(),
    }
}

export!(ChannelPublish);
