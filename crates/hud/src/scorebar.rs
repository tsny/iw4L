use hud_iw4::{Operand, scorebar_gametype_loc_key};

pub(crate) fn milliseconds() -> u32 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();

    const UPTIME_BIAS_MS: u32 = 60_000;
    ORIGIN.get_or_init(Instant::now).elapsed().as_millis() as u32 + UPTIME_BIAS_MS
}

pub(crate) fn gametype_display_name(
    kind: gamemode_iw4::GameModeKind,
    localize: Option<&asset_game::LocalizeCatalog>,
) -> Result<Operand, hud_iw4::ExprError> {
    let key =
        scorebar_gametype_loc_key(kind.token()).ok_or(hud_iw4::ExprError::Host("gametype loc"))?;
    let text = localize
        .and_then(|l| l.text(key))
        .ok_or(hud_iw4::ExprError::Host("gametype localization"))?;
    Ok(Operand::Str(text.to_owned()))
}
