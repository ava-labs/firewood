// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(test)]
mod tests {

    #[test]
    fn test_firewood_counter_macro() {
        // Test counter without labels
        let counter = crate::firewood_counter!("test.counter.simple", "A simple test counter");
        counter.increment(1);

        // Test counter with labels
        let counter_with_labels = crate::firewood_counter!("test.counter.labeled", "A labeled test counter", "env" => "test");
        counter_with_labels.increment(5);
    }

    #[test]
    fn test_firewood_gauge_macro() {
        // Test gauge without labels
        let gauge = crate::firewood_gauge!("test.gauge.simple", "A simple test gauge");
        gauge.set(42.0);
        gauge.increment(10.0);
        gauge.decrement(5.0);

        // Test gauge with labels
        let gauge_with_labels =
            crate::firewood_gauge!("test.gauge.labeled", "A labeled test gauge", "env" => "test");
        gauge_with_labels.set(100.0);
    }

    #[test]
    fn test_macro_description_registration() {
        // Verify that calling the macro multiple times doesn't panic
        // (the static ONCE guard should ensure describe_* is only called once)
        for i in 0..10 {
            let counter = crate::firewood_counter!("test.counter.multi", "Multi-call test counter");
            counter.increment(1);

            let gauge = crate::firewood_gauge!("test.gauge.multi", "Multi-call test gauge");
            gauge.set(f64::from(i));
        }
    }
}
