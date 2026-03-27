use serde_json::Value;

pub(crate) fn print_queue_table(value: &Value) {
    if let (Some(index_items), Some(data_map)) = (
        value.get("index").and_then(Value::as_array),
        value.get("data").and_then(Value::as_object),
    ) {
        print_queue_rows_from_index(index_items, data_map);
        return;
    }

    println!("queueId\tname");
}

fn print_queue_rows_from_index(index_items: &[Value], data_map: &serde_json::Map<String, Value>) {
    println!("queueId\tname");
    for item in index_items {
        let uid = item
            .get("uid")
            .or_else(|| item.get("queueId"))
            .and_then(Value::as_str)
            .unwrap_or("-");

        let queue_name = data_map
            .get(uid)
            .and_then(|entry| entry.get("Info"))
            .and_then(|info| info.get("Name"))
            .and_then(Value::as_str)
            .unwrap_or("-");

        println!("{}\t{}", uid, queue_name);
    }
}
