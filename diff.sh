cargo build --profile maxperf -p firewood --example diff_metrics
for i in {1..10}; do
    ./target/maxperf/examples/diff_metrics --items 10000000 --modify 20000 --db-path diff_db_0 > diff_db_${i}.log
done