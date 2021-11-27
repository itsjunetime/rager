use crate::{err, sync_dir};

const ZSH_INSTALL: &str = "
# For rager view completion
_rager_comp() {
	compadd $(rager complete \"$words[3]\")
}

compdef _rager_comp rager view
";

const BASH_INSTALL: &str = "
# For rager view completion
_rager_comp() {
	COMPREPLY=($(rager complete \"${COMP_WORDS[COMP_CWORD]}\"))
}

complete -o nospace -F _rager_comp rager view
";

pub fn list_completions(input: &str) {
	// for separating the directories
	let sep_char = if cfg!(windows) {
		'\\'
	} else {
		'/'
	};

	let mut dir = sync_dir();

	// iterate through and push them. If we just push them all at once it will consider them
	// one identifier, and then calling `dir.parent()` will return the same thing as
	// `sync_dir()`, which we don't want.
	for part in input.split(sep_char) {
		dir.push(part);
	}

	// if it's there and it's a directory
	if dir.exists() && dir.is_dir() {
		// just iterate through all its contents and print them
		if let Ok(contents) = dir.read_dir() {
			for path in contents.filter_map(|c| c.ok().map(|p| p.path())) {
				// make sure we can get the filename (or directory name)
				// of each of its contents tho
				if let Some(name) = path.file_name().map(|f| f.to_string_lossy()) {
					// and then print it correctly, adding a directory separator if necessary
					if input.is_empty() || input.ends_with(sep_char) {
						println!("{}{}", input, name)
					} else {
						println!("{}{}{}", input, sep_char, name);
					}
				}
			}
		}

		// and then just return so we don't have to do an `else`
		return;
	}

	// make sure it has a parent. Something would be massively broken if it didn't
	// but we don't like unwrapping so we do this. We also have to make sure it exists
	// and is actually a directory
	let parent = match dir.parent() {
		Some(p) if p.exists() && p.is_dir() => p,
		_ => return,
	};

	// and get the filename
	let file_name = match dir.file_name() {
		Some(f) => f.to_string_lossy().to_string(),
		_ => return,
	};

	// and then iterate through the directory again
	if let Ok(contents) = parent.read_dir() {
		for path in contents.filter_map(|c| c.ok().map(|p| p.path())) {
			// and get the name of each
			if let Some(name) = path.file_name().map(|f| f.to_string_lossy()) {
				// and if it matches
				if name.starts_with(&file_name) {
					// grab the remaining part of the name to display
					let name_slice = &name[file_name.len()..];

					// and print it out for completion
					println!("{}{}", input, name_slice);
				}
			}
		}
	}
}

pub fn install_completion() {
	use std::io::Write;

	let mut input = String::new();
	let stdin = std::io::stdin();

	let (file, install_str) = match std::env::var("SHELL") {
		Ok(var) if var.contains("zsh") => (".zshrc", ZSH_INSTALL),
		Ok(var) if var.contains("bash") => (".bashrc", BASH_INSTALL),
		Ok(x) => {
			if x.is_empty() {
				println!("The env var $SHELL is empty; aborting");
			} else {
				println!("Your shell ({}) is currently not supported :(", x);
			}
			return;
		}
		Err(err) => {
			err!("Unable to get value of $SHELL ({}); aborting", err);
			return;
		}
	};

	println!(
		"To install shell completion for rager, we need to append the following lines to your ~/{}:\
		\n{}\
		\nIs that ok? [y/n]",
		file,
		install_str
	);

	stdin
		.read_line(&mut input)
		.expect("Did not enter text correctly");

	if input.to_lowercase().starts_with('y') {
		let mut path = dirs::home_dir().unwrap();
		path.push(file);

		let shell_file = std::fs::OpenOptions::new()
			.append(true)
			.create(true)
			.open(&path);

		match shell_file {
			Ok(mut f) => match f.write(install_str.as_bytes()) {
				Err(err) => err!("Unable to write to file at {:?} ({:?}); are you sure you have the right permissions?", path, err),
				Ok(x) if x > 0 => println!("Successfully installed completion :)\nRun \x1b[1msource ~/{}\x1b[0m to load it in right now.", file),
				_ => err!("Did not install completion successfully; unknown error occured"),
			},
			Err(err) => err!("Unable to open file at {:?}: {}", path, err),
		}
	}
}
