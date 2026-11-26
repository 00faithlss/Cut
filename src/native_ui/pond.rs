use gpui::{prelude::*, *};
const SHARK: &[&str] = &[
    r"                   /|                   ",
    r"             _____/ |_____              ",
    r"        _.-' .::::::::::::`-._          ",
    r"   |\_.' .::::::::::::::::::::`-.       ",
    r"   |  .::::::::::::::::::  o ::: `>     ",
    r"   |/`-. :::::::::::::::::::_.-'        ",
    r"        `-..__________..--'              ",
    r"                 \  |                   ",
    r"                  \ |                   ",
    r"                   \|                   ",
];
pub fn render(phase: f32, ink: u32) -> Div {
    let mut pond = div()
        .relative()
        .w_full()
        .h(px(290.))
        .overflow_hidden()
        .font_family("Consolas")
        .text_size(px(8.))
        .text_color(rgb(ink));
    for i in 0..2 {
        let x = ((phase * 7. + i as f32 * 120.) % 170.) - 85.;
        let y = 22. + i as f32 * 145.;
        pond = pond.child(
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .flex()
                .flex_col()
                .children(SHARK.iter().map(|line| div().h(px(9.)).child(*line))),
        );
    }
    pond
}
