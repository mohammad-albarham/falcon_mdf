use falcon_mdf_gui::percentile::percentile;

#[test]
fn empty_input_has_no_percentile() {
    assert_eq!(percentile(&[], 0.5), None);
}

#[test]
fn a_single_value_is_every_percentile() {
    let values = [42.0];
    assert_eq!(percentile(&values, 0.0), Some(42.0));
    assert_eq!(percentile(&values, 0.5), Some(42.0));
    assert_eq!(percentile(&values, 1.0), Some(42.0));
}

#[test]
fn fraction_zero_is_the_minimum_and_one_is_the_maximum() {
    let values = [3.0, 1.0, 2.0];
    assert_eq!(percentile(&values, 0.0), Some(1.0));
    assert_eq!(percentile(&values, 1.0), Some(3.0));
}

#[test]
fn the_median_of_an_odd_series_is_the_middle_value() {
    let values = [1.0, 2.0, 3.0];
    assert_eq!(percentile(&values, 0.5), Some(2.0));
}

#[test]
fn the_median_of_an_even_series_is_between_the_middle_two() {
    let values = [1.0, 2.0, 3.0, 4.0];
    assert_eq!(percentile(&values, 0.5), Some(2.5));
}

#[test]
fn a_fraction_between_ranks_interpolates() {
    let values = [1.0, 2.0, 3.0, 4.0];
    assert_eq!(percentile(&values, 0.25), Some(1.75));
}

#[test]
fn an_unsorted_input_gives_the_same_answer_and_is_left_alone() {
    let values = vec![9.0, 1.0, 5.0];
    assert_eq!(percentile(&values, 0.5), Some(5.0));
    assert_eq!(values, vec![9.0, 1.0, 5.0]);
}

#[test]
fn a_fraction_outside_the_range_is_clamped() {
    let values = [1.0, 2.0, 3.0];
    assert_eq!(percentile(&values, -1.0), Some(1.0));
    assert_eq!(percentile(&values, 2.0), Some(3.0));
}
