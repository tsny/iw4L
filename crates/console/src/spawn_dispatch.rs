use std::sync::{Arc, RwLock};

use assets::PreparedWeapons;
use bevy::prelude::*;
use net::{ClientActionInbox, LocalPresentClient, PresentedSnapshot};

use crate::{
    ArgCompleter, ConsoleCommand, ConsoleLine, ConsoleRegistry, ConsoleSettings, ConsoleState,
    StaticCompleter,
};

#[derive(Clone)]
struct SpawnCompleter {
    classes: Arc<[String]>,
    weapons: Arc<RwLock<Vec<String>>>,
}

impl ArgCompleter for SpawnCompleter {
    fn complete(&self, prefix: &str) -> Vec<String> {
        StaticCompleter::new(
            self.classes
                .iter()
                .cloned()
                .chain(std::iter::once("weapon".to_owned())),
        )
        .complete(prefix)
    }

    fn complete_with_context(&self, prefix: &str, args: &[&str]) -> Vec<String> {
        match args {
            [] => self.complete(prefix),
            [kind] if kind.eq_ignore_ascii_case("weapon") => self
                .weapons
                .read()
                .map(|weapons| StaticCompleter::new(weapons.iter().cloned()).complete(prefix))
                .unwrap_or_else(|_| Vec::new()),
            _ => Vec::new(),
        }
    }
}

pub(crate) fn register(
    registry: &mut ConsoleRegistry,
    classes: Vec<String>,
    completions: &crate::weapon_dispatch::WeaponArgCompletions,
) {
    if registry.resolve("spawn").is_some() {
        return;
    }
    let completer = SpawnCompleter {
        classes: classes.into(),
        weapons: Arc::clone(&completions.give),
    };
    registry.register(
        crate::CommandSpec::new("spawn")
            .usage("spawn [class] | weapon <weapon> [attachment...] — enter the match or create a world pickup")
            .arg(completer.clone())
            .arg(completer),
    );
}

pub(crate) fn is_world_spawn(command: &ConsoleCommand) -> bool {
    command
        .args
        .first()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("weapon"))
}

pub(crate) fn route(
    mut commands: MessageReader<ConsoleCommand>,
    mut outcomes: MessageReader<net::ReliableControlEvent>,
    mut output: (
        ResMut<ConsoleState>,
        Res<ConsoleSettings>,
        ResMut<ConsoleLine>,
    ),
    weapons: Option<Res<PreparedWeapons>>,
    view: (
        Res<PresentedSnapshot>,
        Res<LocalPresentClient>,
        Option<Res<net::AuthorityWorld>>,
    ),
    mut actions: (
        Option<ResMut<ClientActionInbox>>,
        ResMut<net::ActionRequestIds>,
    ),
) {
    let (console, settings, line) = &mut output;
    let (presented, local, authority) = view;
    let (inbox, sequence) = &mut actions;
    let echo = |message: String, console: &mut ConsoleState, line: &mut ConsoleLine| {
        diag::info!(Console, "{message}");
        line.0 = message.clone();
        console.echo(message, settings.log_capacity);
    };
    for command in commands.read().filter(|command| is_world_spawn(command)) {
        let [_, raw_weapon, attachments @ ..] = command.args.as_slice() else {
            echo(
                "usage: spawn weapon <weapon> [attachment...]".into(),
                console,
                line,
            );
            continue;
        };
        if authority
            .as_ref()
            .is_some_and(|world| !world.0.cheats_enabled())
        {
            echo("spawn weapon: cheats are off".into(), console, line);
            continue;
        }
        let Some(weapons) = weapons.as_ref() else {
            echo(
                "spawn weapon: weapon catalog not loaded".into(),
                console,
                line,
            );
            continue;
        };
        if !presented
            .snapshot()
            .and_then(|snapshot| snapshot.meta.for_client(local.0))
            .is_some_and(|meta| meta.lifecycle == sim::ClientLifecycle::Alive)
        {
            echo(
                "spawn weapon: not Alive — spawn a class first".into(),
                console,
                line,
            );
            continue;
        }
        let weapon =
            match crate::weapon_dispatch::resolve_give_id(&weapons.0, raw_weapon, attachments) {
                Ok(weapon) => weapon,
                Err(error) => {
                    echo(format!("spawn weapon: {error}"), console, line);
                    continue;
                }
            };
        let Some(inbox) = inbox.as_deref_mut() else {
            echo("spawn weapon: no action inbox".into(), console, line);
            continue;
        };
        let request_id = sequence.allocate();
        let action = sim::ClientAction::DebugSpawn {
            request_id,
            recipe: sim::DebugSpawnRecipe::Weapon { weapon },
        };
        match inbox.push(local.0, action) {
            Ok(()) => echo(
                format!(
                    "spawn weapon: queued {} id={weapon} request_id={request_id}",
                    weapons.0.configuration_label(weapon)
                ),
                console,
                line,
            ),
            Err(error) => echo(format!("spawn weapon: {error}"), console, line),
        }
    }
    for outcome in outcomes.read() {
        match outcome.0 {
            sim::SimEvent::DebugSpawnAccepted {
                request_id,
                recipe: sim::DebugSpawnRecipe::Weapon { weapon },
                entity,
                origin,
            } => echo(
                format!(
                    "spawn weapon: ok {} id={weapon} entity={} at={:.1},{:.1},{:.1} request_id={request_id}",
                    weapons
                        .as_ref()
                        .map(|weapons| weapons.0.configuration_label(weapon))
                        .filter(|label| !label.is_empty())
                        .unwrap_or_else(|| format!("#{weapon}")),
                    entity.number(),
                    origin[0],
                    origin[1],
                    origin[2],
                ),
                console,
                line,
            ),
            sim::SimEvent::DebugSpawnRejected {
                request_id,
                recipe: sim::DebugSpawnRecipe::Weapon { weapon },
                reason,
            } => echo(
                format!("spawn weapon: rejected id={weapon} request_id={request_id} ({reason})"),
                console,
                line,
            ),
            _ => {}
        }
    }
}
