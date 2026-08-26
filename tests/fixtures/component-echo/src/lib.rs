wit_bindgen::generate!({
    path: "../../../specs/wit",
    world: "application",
});

struct Echo;

impl exports::hologram::application::guest::Guest for Echo {
    fn run(
        input: Vec<u8>,
    ) -> Result<Vec<u8>, exports::hologram::application::guest::GuestError> {
        if input == b"guest-error" {
            return Err(exports::hologram::application::guest::GuestError {
                code: exports::hologram::application::guest::ErrorCode::Failed,
                message: "fixture failure".to_owned(),
            });
        }
        Ok(input)
    }
}

export!(Echo);
