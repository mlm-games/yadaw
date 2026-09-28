use yadaw::model::group::{
    GroupNode, any_group_soloed, group_index, resolve_group_chain, resolve_track_groups,
};

fn node(id: u64, parent: Option<u64>, volume: f32) -> GroupNode {
    GroupNode {
        id,
        parent,
        volume,
        muted: false,
        solo: false,
    }
}

fn muted(id: u64, parent: Option<u64>, volume: f32) -> GroupNode {
    GroupNode {
        muted: true,
        ..node(id, parent, volume)
    }
}

fn soloed(id: u64, parent: Option<u64>, volume: f32) -> GroupNode {
    GroupNode {
        solo: true,
        ..node(id, parent, volume)
    }
}

#[test]
fn a_track_outside_every_group_is_left_alone() {
    let index = group_index(&[node(1, None, 0.5)]);
    let res = resolve_group_chain(&index, None);
    assert_eq!(res.gain, 1.0, "no group means no attenuation, got {}", res.gain);
    assert!(!res.muted && !res.soloed, "no group means no flags");
    assert!(res.chain.is_empty(), "no group means no chain");
}

#[test]
fn the_group_fader_multiplies_down_the_whole_chain() {
    let index = group_index(&[node(1, None, 0.5), node(2, Some(1), 0.4), node(3, Some(2), 0.5)]);
    let res = resolve_group_chain(&index, Some(3));
    assert!(
        (res.gain - 0.1).abs() < 1e-6,
        "0.5 * 0.4 * 0.5, got {}",
        res.gain
    );
    assert_eq!(
        res.chain.as_slice(),
        &[3, 2, 1],
        "the chain is recorded innermost first so metering attributes correctly"
    );
}

#[test]
fn a_mute_anywhere_up_the_chain_silences_the_track() {
    let index = group_index(&[muted(1, None, 1.0), node(2, Some(1), 1.0)]);
    let res = resolve_group_chain(&index, Some(2));
    assert!(res.muted, "a muted ancestor mutes the whole subtree");

    let index = group_index(&[node(1, None, 1.0), muted(2, Some(1), 1.0)]);
    let res = resolve_group_chain(&index, Some(2));
    assert!(res.muted, "muting the group the track is in is enough");
}

#[test]
fn soloing_a_sibling_group_leaves_this_track_unsoloed() {
    let index = group_index(&[node(1, None, 1.0), soloed(2, None, 1.0)]);
    assert!(
        any_group_soloed(&index),
        "a solo anywhere has to be visible to the engine"
    );
    let res = resolve_group_chain(&index, Some(1));
    assert!(
        !res.soloed,
        "a track in a different group is not covered by that group's solo"
    );
}

#[test]
fn soloing_an_outer_group_covers_the_whole_subtree() {
    let index = group_index(&[soloed(1, None, 1.0), node(2, Some(1), 1.0), node(3, Some(2), 1.0)]);
    assert!(
        resolve_group_chain(&index, Some(2)).soloed,
        "a nested group inherits the solo of its parent"
    );
    assert!(
        resolve_group_chain(&index, Some(3)).soloed,
        "a track two levels down still inherits it"
    );
}

#[test]
fn every_track_gets_its_own_resolution() {
    let index = group_index(&[node(1, None, 0.5), node(2, Some(1), 0.5)]);
    let all = resolve_track_groups(
        &index,
        [(10, Some(2)), (11, None), (12, Some(1))],
    );
    assert!((all[&10].gain - 0.25).abs() < 1e-6, "nested, got {}", all[&10].gain);
    assert_eq!(all[&11].gain, 1.0, "ungrouped, got {}", all[&11].gain);
    assert!((all[&12].gain - 0.5).abs() < 1e-6, "outer, got {}", all[&12].gain);
}

#[test]
fn a_missing_group_costs_the_rest_of_the_chain_not_the_own_group() {
    let index = group_index(&[node(1, Some(99), 0.5)]);
    let res = resolve_group_chain(&index, Some(1));
    assert!(
        (res.gain - 0.5).abs() < 1e-6,
        "the group's own fader still applies, got {}",
        res.gain
    );
    assert_eq!(res.chain.as_slice(), &[1], "climbing stops at the missing parent");
}

#[test]
fn a_parent_cycle_terminates_instead_of_spinning() {
    // A corrupt file can point two groups at each other.
    let index = group_index(&[node(1, Some(2), 0.5), node(2, Some(1), 0.5)]);
    let res = resolve_group_chain(&index, Some(1));
    assert_eq!(res.chain.as_slice(), &[1, 2], "each group is visited once");
    assert!(
        (res.gain - 0.25).abs() < 1e-6,
        "the gain stops folding at the repeat instead of collapsing, got {}",
        res.gain
    );

    let index = group_index(&[node(1, Some(1), 0.5)]);
    let res = resolve_group_chain(&index, Some(1));
    assert_eq!(res.chain.len(), 1, "a group that is its own parent stops at once");
    assert!((res.gain - 0.5).abs() < 1e-6, "and its own fader still applies");
}

#[test]
fn a_broken_group_volume_is_ignored_rather_than_poisoning_the_mix() {
    let index = group_index(&[node(1, None, f32::NAN), node(2, Some(1), -1.0)]);
    let res = resolve_group_chain(&index, Some(2));
    assert_eq!(
        res.gain, 1.0,
        "NaN and negative volumes are treated as unity so the output stays finite"
    );

    let index = group_index(&[node(1, None, f32::INFINITY)]);
    let res = resolve_group_chain(&index, Some(1));
    assert_eq!(res.gain, 1.0, "an infinite volume cannot reach the mix");
}

#[test]
fn a_zero_group_volume_silences_without_being_dropped() {
    let index = group_index(&[node(1, None, 0.0)]);
    let res = resolve_group_chain(&index, Some(1));
    assert_eq!(res.gain, 0.0, "a fader at the bottom is real silence");
    assert_eq!(res.chain.as_slice(), &[1], "and the group is still metered");
}
