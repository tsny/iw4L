use bevy::prelude::*;

use crate::plugin::ConsoleCommandQueue;
use crate::{ConsoleCommand, ConsoleRegistry};

// Runs through the console FIFO because the menu dispatcher has no `wait world`.

const SCRIPT: &str = "map mp_rust; wait world; spawn 0; force_match_start; bot add 5";

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    if registry.resolve("quickmatch").is_some() {
        return;
    }
    registry.register(
        crate::CommandSpec::new("quickmatch")
            .usage("quickmatch — local mp_rust match with 5 bots, no warmup (needs cheats)"),
    );
}

pub(crate) fn route(
    mut events: MessageReader<ConsoleCommand>,
    mut queue: ResMut<ConsoleCommandQueue>,
) {
    for _ in events.read().filter(|command| command.name == "quickmatch") {
        queue.push_script(SCRIPT);
    }
}
