use bevy::prelude::*;
use net::{ClientActionInbox, LocalPresentClient, PresentedSnapshot};

use crate::{
    ConsoleCommand, ConsoleLine, ConsoleRegistry, ConsoleSettings, ConsoleState, StaticCompleter,
};

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    if registry.resolve("loot_grant").is_some() {
        return;
    }
    let keys = sim::loot_catalog()
        .definitions()
        .iter()
        .map(|definition| definition.key.to_string());
    registry.register(
        crate::CommandSpec::new("loot_grant")
            .usage("loot_grant <namespace:item> [quantity] — add authored loot (needs cheats)")
            .arg(StaticCompleter::new(keys)),
    );
}

pub(crate) fn route(
    mut events: MessageReader<ConsoleCommand>,
    mut console: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
    mut line: ResMut<ConsoleLine>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    authority: Option<Res<net::AuthorityWorld>>,
    mut inbox: Option<ResMut<ClientActionInbox>>,
    mut sequence: ResMut<net::ActionRequestIds>,
) {
    let capacity = settings.log_capacity;
    let echo = |message: String, console: &mut ConsoleState, line: &mut ConsoleLine| {
        diag::info!(Console, "{message}");
        line.0 = message.clone();
        console.echo(message, capacity);
    };

    for command in events.read().filter(|command| command.name == "loot_grant") {
        let (key, quantity) = match command.args.as_slice() {
            [key] => (key.as_str(), 1),
            [key, raw_quantity] => match raw_quantity.parse::<u16>() {
                Ok(quantity) if quantity != 0 => (key.as_str(), quantity),
                _ => {
                    echo(
                        "loot_grant: quantity must be an integer in 1..=65535".into(),
                        &mut console,
                        &mut line,
                    );
                    continue;
                }
            },
            _ => {
                echo(
                    "usage: loot_grant <namespace:item> [quantity]".into(),
                    &mut console,
                    &mut line,
                );
                continue;
            }
        };
        let Some(key_field) = sim::loot_key_field(key) else {
            echo(
                format!("loot_grant: invalid item key `{key}`"),
                &mut console,
                &mut line,
            );
            continue;
        };
        let known = sim::loot_catalog()
            .definitions()
            .iter()
            .any(|definition| definition.key.as_str() == key);
        if !known {
            echo(
                format!("loot_grant: unknown item `{key}`"),
                &mut console,
                &mut line,
            );
            continue;
        }
        if authority
            .as_ref()
            .is_some_and(|world| !world.0.cheats_enabled())
        {
            echo("loot_grant: cheats are off".into(), &mut console, &mut line);
            continue;
        }
        let alive = presented
            .snapshot()
            .and_then(|snapshot| snapshot.meta.for_client(local.0))
            .is_some_and(|meta| meta.lifecycle == sim::ClientLifecycle::Alive);
        if !alive {
            echo(
                "loot_grant: not Alive — spawn a class first".into(),
                &mut console,
                &mut line,
            );
            continue;
        }
        let Some(inbox) = inbox.as_deref_mut() else {
            echo(
                "loot_grant: no action inbox".into(),
                &mut console,
                &mut line,
            );
            continue;
        };
        let request_id = sequence.allocate();
        let action = sim::ClientAction::DebugGrantLoot {
            request_id,
            key: key_field,
            quantity,
        };
        if let Err(error) = inbox.push(local.0, action) {
            echo(format!("loot_grant: {error}"), &mut console, &mut line);
            continue;
        }
        echo(
            format!("loot_grant: queued {key} ×{quantity} request_id={request_id}"),
            &mut console,
            &mut line,
        );
    }
}
