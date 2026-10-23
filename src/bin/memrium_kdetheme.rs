//! Writes the shell's text editor themes ("Memrium Light" and "Memrium Dark") for Kate and the other KTextEditor programs
//! to `~/.local/share/org.kde.syntax-highlighting/themes`, in the accent of the current theme. A Memrium session does this itself;
//! this is for trying them out in another desktop. Choose one in Kate: Settings > Configure Kate > Editor Component > Fonts & Colors.

fn main() {
    let theme = memrium_de::theme::Config::load().resolve();
    for f in memrium_de::kdecolors::write_editor_themes(theme.accent) {
        println!("wrote {}", f.display());
    }
}
