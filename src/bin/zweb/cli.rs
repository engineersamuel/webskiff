use crate::render::Gfx;

pub const HELP: &str = "\
webskiff - a keyboard-first Chromium browser for your terminal

Usage: webskiff [OPTIONS] [URL-OR-SEARCH]

Options:
  --gfx kitty|shm|blocks  Override terminal graphics detection
  --scale 1|2           Override pixel scale
  --placeholders        Replace page images with flat boxes
  --selftest            Run headless engine checks
  -V, --version         Show application, engine, target, and build versions
  -h, --help            Show this help
  --                    Treat remaining text as the address or search

Quote a search or file path containing spaces.
In the browser: ? or F1 shows shortcuts; Ctrl-Q quits.
Configuration: ~/.config/webskiff/config.toml";

#[derive(Debug)]
pub enum Command {
    Help,
    Version,
    Selftest,
    Browse {
        address: Option<String>,
        gfx: Option<Gfx>,
        scale: Option<u32>,
        placeholders: bool,
    },
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let (mut address, mut gfx, mut scale, mut placeholders) = (None, None, None, false);
    let mut command = None;
    let mut positional = false;
    while let Some(arg) = args.next() {
        if !positional {
            match arg.as_str() {
                "--" => {
                    positional = true;
                    continue;
                }
                "-h" | "--help" => command = Some(Command::Help),
                "-V" | "--version" => command = Some(Command::Version),
                "--selftest" => command = Some(Command::Selftest),
                "--placeholders" => placeholders = true,
                "--gfx" => {
                    gfx = Some(parse_graphics(args.next().as_deref())?);
                }
                "--scale" => {
                    scale = Some(parse_scale(args.next().as_deref())?);
                }
                _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
                _ => {
                    if address.replace(arg).is_some() {
                        return Err("only one address or search is allowed; quote spaces".into());
                    }
                }
            }
        } else if address.replace(arg).is_some() {
            return Err("only one address or search is allowed; quote spaces".into());
        }
    }
    Ok(command.unwrap_or(Command::Browse {
        address,
        gfx,
        scale,
        placeholders,
    }))
}

fn parse_graphics(value: Option<&str>) -> Result<Gfx, String> {
    match value {
        Some("kitty") => Ok(Gfx::KittyDirect),
        Some("shm") => Ok(Gfx::KittyShm),
        Some("blocks") => Ok(Gfx::Blocks),
        _ => Err("--gfx requires kitty, shm, or blocks".into()),
    }
}

fn parse_scale(value: Option<&str>) -> Result<u32, String> {
    match value {
        Some("1") => Ok(1),
        Some("2") => Ok(2),
        _ => Err("--scale requires 1 or 2".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn parses_options_on_either_side_of_an_address() {
        assert!(matches!(
            parsed(&["--gfx", "kitty", "two words", "--scale", "2", "--placeholders"]).unwrap(),
            Command::Browse {
                address: Some(address), gfx: Some(Gfx::KittyDirect), scale: Some(2), placeholders: true
            } if address == "two words"
        ));
        assert!(matches!(
            parsed(&["--", "-a-file.html"]).unwrap(),
            Command::Browse { address: Some(address), .. } if address == "-a-file.html"
        ));
    }

    #[test]
    fn rejects_invalid_options_before_terminal_startup() {
        for args in [
            vec!["--gfx"],
            vec!["--gfx", "invalid"],
            vec!["--scale"],
            vec!["--scale", "0"],
            vec!["--scale", "3"],
            vec!["--typo"],
            vec!["one", "two"],
        ] {
            assert!(parsed(&args).is_err(), "{args:?}");
        }
        assert!(matches!(parsed(&["--version"]).unwrap(), Command::Version));
        assert!(matches!(parsed(&["--help"]).unwrap(), Command::Help));
    }
}
