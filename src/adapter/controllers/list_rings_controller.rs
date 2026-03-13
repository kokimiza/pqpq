use crate::application::dto::request::ListRingsRequest;
use crate::application::ports::input::ListRingsInputPort;
use anyhow::Result;

pub struct ListRingsController<I: ListRingsInputPort> {
    input_port: I,
}

impl<I: ListRingsInputPort> ListRingsController<I> {
    pub fn new(input_port: I) -> Self {
        Self { input_port }
    }

    pub async fn execute(&self) -> Result<()> {
        let request = ListRingsRequest;
        self.input_port.execute(request).await
    }
}
