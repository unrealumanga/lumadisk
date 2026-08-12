use eframe::egui::{Rect, Vec2};

pub fn layout(weights: &[(usize, u64)], rect: Rect) -> Vec<(usize, Rect)> {
    let mut output = Vec::with_capacity(weights.len());
    if !weights.is_empty() && rect.width() > 1.0 && rect.height() > 1.0 {
        partition(weights, rect, &mut output);
    }
    output
}

fn partition(weights: &[(usize, u64)], rect: Rect, output: &mut Vec<(usize, Rect)>) {
    if weights.len() == 1 {
        output.push((weights[0].0, rect));
        return;
    }

    let total: u128 = weights
        .iter()
        .map(|(_, value)| (*value).max(1) as u128)
        .sum();
    let half = total / 2;
    let mut running = 0_u128;
    let mut split = 1_usize;
    for (position, (_, value)) in weights.iter().enumerate().take(weights.len() - 1) {
        running += (*value).max(1) as u128;
        split = position + 1;
        if running >= half {
            break;
        }
    }
    let ratio = (running as f64 / total as f64).clamp(0.02, 0.98) as f32;

    let (first_rect, second_rect) = if rect.width() >= rect.height() {
        let cut = rect.left() + rect.width() * ratio;
        (
            Rect::from_min_max(rect.min, eframe::egui::pos2(cut, rect.bottom())),
            Rect::from_min_max(eframe::egui::pos2(cut, rect.top()), rect.max),
        )
    } else {
        let cut = rect.top() + rect.height() * ratio;
        (
            Rect::from_min_max(rect.min, eframe::egui::pos2(rect.right(), cut)),
            Rect::from_min_max(eframe::egui::pos2(rect.left(), cut), rect.max),
        )
    };

    partition(&weights[..split], inset(first_rect, 0.6), output);
    partition(&weights[split..], inset(second_rect, 0.6), output);
}

pub fn inset(rect: Rect, amount: f32) -> Rect {
    rect.shrink2(Vec2::splat(
        amount.min(rect.width() / 3.0).min(rect.height() / 3.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, pos2};

    #[test]
    fn lays_out_each_item_inside_the_container() {
        let container = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 500.0));
        let output = layout(&[(0, 500), (1, 300), (2, 200)], container);

        assert_eq!(output.len(), 3);
        for (_, rect) in output {
            assert!(container.contains(rect.min));
            assert!(container.contains(rect.max));
            assert!(rect.width() > 0.0 && rect.height() > 0.0);
        }
    }
}
