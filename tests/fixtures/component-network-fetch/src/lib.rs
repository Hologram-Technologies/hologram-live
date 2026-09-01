wit_bindgen::generate!({
    path: "../../../specs/wit/network-fetch",
    world: "application",
    generate_all,
});

struct NetworkFetch;

impl exports::hologram::application_network_fetch::guest::Guest for NetworkFetch {
    fn run(
        input: Vec<u8>,
    ) -> Result<Vec<u8>, exports::hologram::application_network_fetch::guest::GuestError> {
        let target = String::from_utf8(input).map_err(|_| invalid_input("target must be UTF-8"))?;
        let response = hologram::host::network_fetch::fetch(&target).map_err(|message| {
            exports::hologram::application_network_fetch::guest::GuestError {
                code: exports::hologram::application_network_fetch::guest::ErrorCode::Failed,
                message,
            }
        })?;
        let mut output = response.status.to_be_bytes().to_vec();
        output.extend_from_slice(&response.body);
        Ok(output)
    }
}

fn invalid_input(
    message: &str,
) -> exports::hologram::application_network_fetch::guest::GuestError {
    exports::hologram::application_network_fetch::guest::GuestError {
        code: exports::hologram::application_network_fetch::guest::ErrorCode::InvalidInput,
        message: message.to_owned(),
    }
}

export!(NetworkFetch);
