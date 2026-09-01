wit_bindgen::generate!({
    path: "../../../specs/wit/store-write",
    world: "application",
    generate_all,
});

struct StoreWrite;

impl exports::hologram::application_store_write::guest::Guest for StoreWrite {
    fn run(
        input: Vec<u8>,
    ) -> Result<Vec<u8>, exports::hologram::application_store_write::guest::GuestError> {
        let Some(separator) = input.iter().position(|byte| *byte == b'\n') else {
            return Err(invalid_input("expected a kappa line followed by object bytes"));
        };
        let kappa = String::from_utf8(input[..separator].to_vec())
            .map_err(|_| invalid_input("kappa must be UTF-8"))?;
        hologram::host::store_write::write(&kappa, &input[separator + 1..]).map_err(|message| {
            exports::hologram::application_store_write::guest::GuestError {
                code: exports::hologram::application_store_write::guest::ErrorCode::Failed,
                message,
            }
        })?;
        Ok(kappa.into_bytes())
    }
}

fn invalid_input(
    message: &str,
) -> exports::hologram::application_store_write::guest::GuestError {
    exports::hologram::application_store_write::guest::GuestError {
        code: exports::hologram::application_store_write::guest::ErrorCode::InvalidInput,
        message: message.to_owned(),
    }
}

export!(StoreWrite);
