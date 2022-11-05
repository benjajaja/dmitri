use breadx::{
    display::{DisplayConnection, DisplayFunctionsExt},
    prelude::*,
    protocol::{
        xproto::{self, EventMask, InputFocus, SetInputFocusRequest, UngrabKeyRequest},
        Event,
    },
};
use breadx_keysyms::{keysyms, KeyboardState};
use getopts::Options;
use hex_color::HexColor;
use rust_fuzzy_search::fuzzy_search_best_n;
use std::{boxed::Box, env, error::Error, fs, os::unix::prelude::MetadataExt, process};

mod text;
use text::{FontRenderer, RunOptions};

fn main() -> Result<(), Box<dyn Error>> {
    let mut opts = Options::new();
    opts.optopt("f", "fontname", "set font name", "mono");
    opts.optopt("s", "fontsize", "set font size", "32");
    opts.optopt("m", "margin", "set margin", "7");
    opts.optopt("c", "color", "set color", "#ff8800");
    opts.optopt(
        "p",
        "precise-wheight",
        "set additional wheight of subtext matching",
        "5.0",
    );

    opts.optflag("h", "help", "print this help menu");

    let args: Vec<String> = std::env::args().collect();
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!("Could not parse arguments: {}", f),
    };
    if matches.opt_present("h") {
        println!("{}", opts.usage("dmitri: a launcher"));
        return Ok(());
    }
    let options = RunOptions {
        fontname: matches.opt_str("f"),
        fontsize: matches
            .opt_str("s")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(32),
        color: text::color_from_u8(
            matches
                .opt_str("c")
                .and_then(|s| s.parse::<HexColor>().ok())
                .map(|h| (h.r, h.g, h.b))
                .unwrap_or((255, 127, 0)),
        ),
        margin: matches
            .opt_str("m")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(7),
        precise_wheight: matches
            .opt_str("p")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(5.0),
    };

    let mut conn = DisplayConnection::connect(None)?;

    let root = conn.default_screen().root;
    //
    // let cookie = conn.send_request(GetInputFocusRequest {
    // ..Default::default()
    // })?;
    // let reply = conn.resolve_request(cookie)?;
    // let focus_window = reply.focus;
    //
    // let screens = conn.screens().to_owned();
    // 'out: for screen in screens {
    // let tree = screen.root.query_tree_immediate(&mut conn)?;
    // for child in tree.children.iter() {
    // if *child == focus_window {
    // println!("it is child");
    // root = screen.root;
    // break 'out;
    // }
    // }
    // }

    let root_geometry = conn.get_geometry_immediate(root)?;

    let height = options.fontsize + (options.margin * 2) as u16;

    let wid = conn.generate_xid()?;
    conn.create_window_checked(
        0, // depth
        wid,
        root,                // parent
        0,                   // x
        0,                   // y
        root_geometry.width, // width
        height,              // height
        0,                   // border width
        xproto::WindowClass::COPY_FROM_PARENT,
        0, // visual
        xproto::CreateWindowAux::new()
            .background_pixel(conn.default_screen().black_pixel)
            .override_redirect(1)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::KEY_PRESS
                    | EventMask::KEY_RELEASE
                    | EventMask::VISIBILITY_CHANGE
                    | EventMask::FOCUS_CHANGE,
            ),
    )?;

    conn.map_window(wid)?;
    // window.set_title(&mut conn, "Hello World!")?;

    conn.send_void_request(
        SetInputFocusRequest {
            focus: wid,
            revert_to: InputFocus::PARENT,
            ..Default::default()
        },
        true,
    )?;

    match run(&mut conn, wid, root, options) {
        Err(err) => {
            eprintln!("Error: {}", err);
            Err(err)
        }
        Ok(output) => {
            if !output.is_empty() {
                return spawn(output);
            }
            Ok(())
        }
    }
}

fn run<Dpy: Display>(
    connection: &mut Dpy,
    wid: u32,
    root: u32,
    options: RunOptions,
) -> Result<String, Box<dyn Error>> {
    let gc = connection.generate_xid()?;
    connection.create_gc_checked(
        gc,
        wid,
        xproto::CreateGCAux::new()
            .foreground(connection.default_screen().black_pixel)
            .graphics_exposures(0)
            .line_width(10),
    )?;

    let geometry = connection.get_geometry_immediate(wid)?;
    let mut font_render = FontRenderer::new(
        connection,
        geometry.depth,
        geometry.width as _,
        geometry.height as _,
        &options,
    )?;
    let mut input = String::new();

    let mut matches: Vec<String> = vec![];
    let mut matches_i: Option<usize> = None;

    font_render.render_text(connection, wid, gc, "█", &matches, matches_i)?;

    // set up an exit strategy
    let wm_protocols = connection.intern_atom(false, "WM_PROTOCOLS")?;
    let wm_delete_window = connection.intern_atom(false, "WM_DELETE_WINDOW")?;
    connection.flush()?;
    let wm_protocols = connection.wait_for_reply(wm_protocols)?.atom;
    let wm_delete_window = connection.wait_for_reply(wm_delete_window)?.atom;

    connection.change_property(
        xproto::PropMode::REPLACE,
        wid,
        wm_protocols,
        xproto::AtomEnum::ATOM.into(),
        32,
        1,
        &wm_delete_window,
    )?;

    let mut keystate = KeyboardState::new(connection)?;
    let mut is_shift = false;

    let executables = build_path()?;

    loop {
        let ev = match connection.wait_for_event() {
            Ok(ev) => ev,
            Err(e) => {
                eprintln!("Program closed with error: {:?}", e);
                process::exit(1);
            }
        };

        match ev {
            Event::ClientMessage(cme) => {
                if cme.data.as_data32()[0] == wm_delete_window {
                    process::exit(0);
                }
            }
            Event::Expose(_) => {
                font_render.render_text(connection, wid, gc, &input, &matches, matches_i)?;
            }
            Event::FocusOut(_e) => {
                connection.send_void_request(
                    SetInputFocusRequest {
                        focus: wid,
                        revert_to: InputFocus::PARENT,
                        ..Default::default()
                    },
                    true,
                )?;
            }
            Event::KeyPress(kp) => {
                let sym = keystate.symbol(connection, kp.detail, 0)?;
                match sym {
                    keysyms::KEY_Escape => {
                        connection.send_void_request(
                            UngrabKeyRequest {
                                grab_window: root,
                                ..Default::default()
                            },
                            true,
                        )?;
                        connection.unmap_window(wid)?;
                        // window.free(conn)?;
                        return Ok(String::new());
                    }
                    keysyms::KEY_Return => {
                        let output: String = match matches_i {
                            None => input,
                            Some(i) => matches.get(i).map(String::to_owned).unwrap_or(input),
                        };
                        return Ok(output);
                    }
                    keysyms::KEY_Tab => {
                        if matches.len() > 1 {
                            match matches_i {
                                None => {
                                    if !is_shift {
                                        matches_i = Some(0);
                                    } else {
                                        matches_i = Some(matches.len() - 1);
                                    }
                                }
                                Some(i) => {
                                    if !is_shift {
                                        match matches.get(i + 1) {
                                            Some(_) => matches_i = Some(i + 1),
                                            None => matches_i = None,
                                        }
                                    } else if i > 0 && matches.get(i - 1).is_some() {
                                        matches_i = Some(i - 1);
                                    } else {
                                        matches_i = None;
                                    }
                                }
                            }
                        }
                    }
                    keysyms::KEY_BackSpace => {
                        if !input.is_empty() {
                            input = input[0..input.len() - 1].to_string();
                            matches_i = None;
                            matches = search(&input, &executables, options.precise_wheight);
                        }
                    }
                    keysyms::KEY_Shift_L | keysyms::KEY_Shift_R => {
                        is_shift = true;
                    }
                    k => {
                        if let Some(mut keycode_char) = char::from_u32(k) {
                            keycode_char = keycode_char
                                .to_lowercase()
                                .next()
                                .ok_or("lowercase keycode char")?;
                            input.push(keycode_char);
                            matches_i = None;
                            matches = search(&input, &executables, options.precise_wheight);
                        }
                    }
                }
                font_render.render_text(connection, wid, gc, &input, &matches, matches_i)?;
            }
            Event::KeyRelease(kr) => {
                let sym = keystate.symbol(connection, kr.detail, 0)?;
                match sym {
                    keysyms::KEY_Shift_L | keysyms::KEY_Shift_R => {
                        is_shift = false;
                    }
                    _ => (),
                }
            }
            _ => (),
        }
    }
}

fn build_path() -> Result<Vec<String>, Box<dyn Error>> {
    let mut executables: Vec<String> = vec![];

    let path_var = env::var("PATH")?;
    let paths = path_var.split(':');
    for path in paths {
        if let Ok(dir) = fs::read_dir(path) {
            for entry in dir {
                let entry = entry?;

                let os_filename = entry.file_name();
                let filename = os_filename.to_string_lossy().to_string();
                if executables.contains(&filename) {
                    continue;
                }
                let pathbuf = entry.path();
                let metadata = fs::metadata(&pathbuf)?;
                if !metadata.is_file() {
                    continue;
                }
                if metadata.mode() & 0o111 != 0 {
                    executables.push(filename);
                }
            }
        }
    }
    executables.sort();
    Ok(executables)
}

fn search(input: &String, executables: &[String], precise_wheight: f32) -> Vec<String> {
    if input.is_empty() {
        return vec![];
    }

    let list = executables
        .iter()
        .map(String::as_ref)
        .collect::<Vec<&str>>();

    let mut res: Vec<(&str, f32)> = fuzzy_search_best_n(input, &list, 20);
    for (entry, i) in &mut res {
        if let Some(start) = entry.find(input) {
            *i += (precise_wheight / (start as f32 + precise_wheight)) as f32;
        }
    }
    res.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    return res.iter().map(|(s, _)| String::from(*s)).collect();
}

fn spawn(output: String) -> Result<(), Box<dyn Error>> {
    if let Err(err) = process::Command::new(output).spawn() {
        eprintln!("Command error: {}", err);
    }
    Ok(())
}
