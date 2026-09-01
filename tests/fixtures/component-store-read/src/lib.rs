wit_bindgen::generate!({
    path: "../../../specs/wit/store-read",
    world: "application",
    generate_all,
});

struct StoreRead;

impl exports::hologram::application_store_read::guest::Guest for StoreRead {
    fn run(
        input: Vec<u8>,
    ) -> Result<Vec<u8>, exports::hologram::application_store_read::guest::GuestError> {
        let kappa = String::from_utf8(input).map_err(|error| {
            exports::hologram::application_store_read::guest::GuestError {
                code: exports::hologram::application_store_read::guest::ErrorCode::InvalidInput,
                message: error.to_string(),
            }
        })?;
        hologram::host::store::read(&kappa).map_err(|message| {
            exports::hologram::application_store_read::guest::GuestError {
                code: exports::hologram::application_store_read::guest::ErrorCode::Failed,
                message,
            }
        })
    }
}

export!(StoreRead);
