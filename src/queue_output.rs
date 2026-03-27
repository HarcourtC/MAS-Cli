use serde_json::Value;

pub(crate) fn print_queue_table(value: &Value) {
    if let Some(data) = value.get("data") {
        if let Some(queue_items) = data.get("list").and_then(|v| v.as_array()) {
            print_queue_rows(queue_items);
            return;
        }
    }

    if let Some(queue_items) = value.as_array() {
        print_queue_rows(queue_items);
        return;
    }

    println!("queueId\tname");
}

fn print_queue_rows(queue_items: &[Value]) {
    println!("queueId\tname");
    for item in queue_items {
        let queue_id = item.get("queueId").and_then(Value::as_str).unwrap_or("-");
        let queue_name = item
            .get("queueName")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        println!("{}\t{}", queue_id, queue_name);
    }
}
