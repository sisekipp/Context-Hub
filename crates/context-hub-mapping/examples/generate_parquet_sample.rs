use std::{error::Error, fs::File, sync::Arc};

use datafusion::arrow::{
    array::{ArrayRef, BooleanArray, Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use parquet::arrow::ArrowWriter;

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "context-hub-parquet-sample.parquet".into());
    let schema = Arc::new(Schema::new(vec![
        Field::new("service_id", DataType::Utf8, false),
        Field::new("service_name", DataType::Utf8, false),
        Field::new("owner_team", DataType::Utf8, false),
        Field::new("monthly_cost_eur", DataType::Float64, false),
        Field::new("is_active", DataType::Boolean, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["parquet-billing", "parquet-search"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["Parquet Billing", "Parquet Search"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["Payments", "Platform"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![1250.50, 875.25])) as ArrayRef,
            Arc::new(BooleanArray::from(vec![true, true])) as ArrayRef,
        ],
    )?;
    let file = File::create(&output)?;
    let mut writer = ArrowWriter::try_new(file, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;
    println!("wrote {} records to {output}", batch.num_rows());
    Ok(())
}
