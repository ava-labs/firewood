mod tests {
    use coarsetime_metrics_macros::measure;
    use metrics::counter;

    #[test]
    #[measure(name = "xyz")]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
