use actix::prelude::*;

#[derive(Default)]
pub struct Filer {}

impl Actor for Filer {
    type Context = Context<Self>;
}
