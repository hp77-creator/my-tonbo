use std::{any::Any, marker::PhantomData, mem, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, ArrowPrimitiveType, AsArray},
    datatypes::{
        Float32Type, Float64Type, Int16Type, Int32Type, Int64Type, Int8Type, Schema as ArrowSchema,
        UInt16Type, UInt32Type, UInt64Type, UInt8Type,
    },
};
use fusio::Write;
use fusio_log::Encode;

use super::{DataType, DynRecord, Value};
use crate::{
    magic::USER_COLUMN_OFFSET,
    record::{
        option::OptionRecordRef, Key, Record, RecordEncodeError, RecordRef, Schema, F32, F64,
    },
};

#[derive(Clone)]
pub struct DynRecordRef<'r> {
    pub columns: Vec<Value>,
    // XXX: log encode should keep the same behavior
    pub primary_index: usize,
    _marker: PhantomData<&'r ()>,
}

impl<'r> DynRecordRef<'r> {
    pub(crate) fn new(columns: Vec<Value>, primary_index: usize) -> Self {
        Self {
            columns,
            primary_index,
            _marker: PhantomData,
        }
    }
}

impl<'r> Encode for DynRecordRef<'r> {
    type Error = RecordEncodeError;

    async fn encode<W>(&self, writer: &mut W) -> Result<(), Self::Error>
    where
        W: Write,
    {
        (self.columns.len() as u32).encode(writer).await?;
        (self.primary_index as u32).encode(writer).await?;
        for col in self.columns.iter() {
            col.encode(writer).await.map_err(RecordEncodeError::Fusio)?;
        }
        Ok(())
    }

    fn size(&self) -> usize {
        let mut size = 2 * mem::size_of::<u32>();
        for col in self.columns.iter() {
            size += col.size();
        }
        size
    }
}

impl<'r> RecordRef<'r> for DynRecordRef<'r> {
    type Record = DynRecord;

    fn key(self) -> <<<Self::Record as Record>::Schema as Schema>::Key as Key>::Ref<'r> {
        self.columns
            .get(self.primary_index)
            .cloned()
            .expect("The primary key must exist")
    }

    fn from_record_batch(
        record_batch: &'r arrow::array::RecordBatch,
        offset: usize,
        projection_mask: &'r parquet::arrow::ProjectionMask,
        full_schema: &'r Arc<ArrowSchema>,
    ) -> OptionRecordRef<'r, Self> {
        let null = record_batch.column(0).as_boolean().value(offset);
        let metadata = full_schema.metadata();

        let primary_index = metadata
            .get("primary_key_index")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let ts = record_batch
            .column(1)
            .as_primitive::<arrow::datatypes::UInt32Type>()
            .value(offset)
            .into();

        let mut columns = vec![];

        for (idx, field) in full_schema.flattened_fields().iter().enumerate().skip(2) {
            let datatype = DataType::from(field.data_type());
            let schema = record_batch.schema();
            let flattened_fields = schema.flattened_fields();
            let batch_field = flattened_fields
                .iter()
                .enumerate()
                .find(|(_idx, f)| field.contains(f));
            if batch_field.is_none() {
                columns.push(Value::with_none_value(
                    datatype,
                    field.name().to_owned(),
                    field.is_nullable(),
                ));
                continue;
            }
            let col = record_batch.column(batch_field.unwrap().0);
            let is_nullable = field.is_nullable();
            let value = match datatype {
                DataType::UInt8 => Self::primitive_value::<UInt8Type>(
                    col,
                    offset,
                    idx,
                    projection_mask,
                    primary_index == idx - 2,
                ),
                DataType::UInt16 => Self::primitive_value::<UInt16Type>(
                    col,
                    offset,
                    idx,
                    projection_mask,
                    primary_index == idx - 2,
                ),
                DataType::UInt32 => Self::primitive_value::<UInt32Type>(
                    col,
                    offset,
                    idx,
                    projection_mask,
                    primary_index == idx - 2,
                ),
                DataType::UInt64 => Self::primitive_value::<UInt64Type>(
                    col,
                    offset,
                    idx,
                    projection_mask,
                    primary_index == idx - 2,
                ),
                DataType::Int8 => Self::primitive_value::<Int8Type>(
                    col,
                    offset,
                    idx,
                    projection_mask,
                    primary_index == idx - 2,
                ),
                DataType::Int16 => Self::primitive_value::<Int16Type>(
                    col,
                    offset,
                    idx,
                    projection_mask,
                    primary_index == idx - 2,
                ),
                DataType::Int32 => Self::primitive_value::<Int32Type>(
                    col,
                    offset,
                    idx,
                    projection_mask,
                    primary_index == idx - 2,
                ),
                DataType::Int64 => Self::primitive_value::<Int64Type>(
                    col,
                    offset,
                    idx,
                    projection_mask,
                    primary_index == idx - 2,
                ),
                DataType::Float32 => {
                    let v = col.as_primitive::<Float32Type>();

                    if primary_index == idx - 2 {
                        Arc::new(F32::from(v.value(offset))) as Arc<dyn Any + Send + Sync>
                    } else {
                        let value = (!v.is_null(offset) && projection_mask.leaf_included(idx))
                            .then_some(F32::from(v.value(offset)));
                        Arc::new(value) as Arc<dyn Any + Send + Sync>
                    }
                }
                DataType::Float64 => {
                    let v = col.as_primitive::<Float64Type>();

                    if primary_index == idx - 2 {
                        Arc::new(F64::from(v.value(offset))) as Arc<dyn Any + Send + Sync>
                    } else {
                        let value = (!v.is_null(offset) && projection_mask.leaf_included(idx))
                            .then_some(F64::from(v.value(offset)));
                        Arc::new(value) as Arc<dyn Any + Send + Sync>
                    }
                }
                DataType::String => {
                    let v = col.as_string::<i32>();

                    if primary_index == idx - 2 {
                        Arc::new(v.value(offset).to_owned()) as Arc<dyn Any + Send + Sync>
                    } else {
                        let value = (!v.is_null(offset) && projection_mask.leaf_included(idx))
                            .then_some(v.value(offset).to_owned());
                        Arc::new(value) as Arc<dyn Any + Send + Sync>
                    }
                }
                DataType::Boolean => {
                    let v = col.as_boolean();

                    if primary_index == idx - 2 {
                        Arc::new(v.value(offset).to_owned()) as Arc<dyn Any + Send + Sync>
                    } else {
                        let value = (!v.is_null(offset) && projection_mask.leaf_included(idx))
                            .then_some(v.value(offset).to_owned());
                        Arc::new(value) as Arc<dyn Any + Send + Sync>
                    }
                }
                DataType::Bytes => {
                    let v = col.as_binary::<i32>();
                    if primary_index == idx - 2 {
                        Arc::new(v.value(offset).to_owned()) as Arc<dyn Any + Send + Sync>
                    } else {
                        let value = (!v.is_null(offset) && projection_mask.leaf_included(idx))
                            .then_some(v.value(offset).to_owned());
                        Arc::new(value) as Arc<dyn Any + Send + Sync>
                    }
                }
            };
            columns.push(Value::new(
                datatype,
                field.name().to_owned(),
                value,
                is_nullable,
            ));
        }

        let record = DynRecordRef {
            columns,
            primary_index,
            _marker: PhantomData,
        };
        OptionRecordRef::new(ts, record, null)
    }

    fn projection(&mut self, projection_mask: &parquet::arrow::ProjectionMask) {
        for (idx, col) in self.columns.iter_mut().enumerate() {
            if idx != self.primary_index && !projection_mask.leaf_included(idx + USER_COLUMN_OFFSET)
            {
                match col.datatype() {
                    DataType::UInt8 => col.value = Arc::<Option<u8>>::new(None),
                    DataType::UInt16 => col.value = Arc::<Option<u16>>::new(None),
                    DataType::UInt32 => col.value = Arc::<Option<u32>>::new(None),
                    DataType::UInt64 => col.value = Arc::<Option<u64>>::new(None),
                    DataType::Int8 => col.value = Arc::<Option<i8>>::new(None),
                    DataType::Int16 => col.value = Arc::<Option<i16>>::new(None),
                    DataType::Int32 => col.value = Arc::<Option<i32>>::new(None),
                    DataType::Int64 => col.value = Arc::<Option<i64>>::new(None),
                    DataType::Float32 => col.value = Arc::<Option<F32>>::new(None),
                    DataType::Float64 => col.value = Arc::<Option<F64>>::new(None),
                    DataType::String => col.value = Arc::<Option<String>>::new(None),
                    DataType::Boolean => col.value = Arc::<Option<bool>>::new(None),
                    DataType::Bytes => col.value = Arc::<Option<Vec<u8>>>::new(None),
                };
            }
        }
    }
}

impl<'r> DynRecordRef<'r> {
    fn primitive_value<T>(
        col: &ArrayRef,
        offset: usize,
        idx: usize,
        projection_mask: &'r parquet::arrow::ProjectionMask,
        primary: bool,
    ) -> Arc<dyn Any + Send + Sync>
    where
        T: ArrowPrimitiveType,
    {
        let v = col.as_primitive::<T>();

        if primary {
            Arc::new(v.value(offset)) as Arc<dyn Any + Send + Sync>
        } else {
            let value = (!v.is_null(offset) && projection_mask.leaf_included(idx))
                .then_some(v.value(offset));
            Arc::new(value) as Arc<dyn Any + Send + Sync>
        }
    }
}

#[cfg(test)]
mod tests {
    use parquet::arrow::{ArrowSchemaConverter, ProjectionMask};

    use crate::{
        cast_arc_value, dyn_record, dyn_schema,
        record::{Record, RecordRef, Schema, F32, F64},
    };

    #[test]
    fn test_float_projection() {
        let schema = dyn_schema!(
            ("_null", Boolean, false),
            ("ts", UInt32, false),
            ("id", Float64, false),
            ("foo", Float32, false),
            ("foo_opt", Float32, true),
            ("bar", Float64, false),
            ("bar_opt", Float64, true),
            2
        );
        let record = dyn_record!(
            ("_null", Boolean, false, true),
            ("ts", UInt32, false, 7u32),
            ("id", Float64, false, F64::from(1.23)),
            ("foo", Float32, false, F32::from(1.23)),
            ("foo_opt", Float32, true, None::<F32>),
            ("bar", Float64, false, F64::from(3.234)),
            ("bar_opt", Float64, true, Some(F64::from(13.234))),
            2
        );
        {
            // test project all
            let mut record_ref = record.as_record_ref();
            record_ref.projection(&ProjectionMask::all());
            let columns = record_ref.columns;
            assert_eq!(*cast_arc_value!(columns[0].value, Option<bool>), Some(true));
            assert_eq!(*cast_arc_value!(columns[1].value, Option<u32>), Some(7u32));
            assert_eq!(*cast_arc_value!(columns[2].value, F64), 1.23.into());
            assert_eq!(
                *cast_arc_value!(columns[3].value, Option<F32>),
                Some(1.23.into())
            );
            assert_eq!(*cast_arc_value!(columns[4].value, Option<F32>), None,);
            assert_eq!(
                *cast_arc_value!(columns[5].value, Option<F64>),
                Some(3.234.into())
            );
            assert_eq!(
                *cast_arc_value!(columns[6].value, Option<F64>),
                Some(13.234.into())
            );
        }
        {
            // test project no columns
            let mut record_ref = record.as_record_ref();
            let mask = ProjectionMask::roots(
                &ArrowSchemaConverter::new()
                    .convert(schema.arrow_schema())
                    .unwrap(),
                vec![1],
            );
            record_ref.projection(&mask);
            let columns = record_ref.columns;
            assert_eq!(*cast_arc_value!(columns[0].value, Option<bool>), None);
            assert_eq!(*cast_arc_value!(columns[1].value, Option<u32>), None);
            assert_eq!(*cast_arc_value!(columns[2].value, F64), 1.23.into());
            assert_eq!(*cast_arc_value!(columns[3].value, Option<F32>), None);
            assert_eq!(*cast_arc_value!(columns[4].value, Option<F32>), None);
            assert_eq!(*cast_arc_value!(columns[5].value, Option<F64>), None);
            assert_eq!(*cast_arc_value!(columns[6].value, Option<F64>), None);
        }
    }

    #[test]
    fn test_string_projection() {
        let schema = dyn_schema!(
            ("_null", Boolean, false),
            ("ts", UInt32, false),
            ("id", String, false),
            ("name", String, false),
            ("email", String, true),
            ("adress", String, true),
            ("data", Bytes, true),
            2
        );
        let record = dyn_record!(
            ("_null", Boolean, false, true),
            ("ts", UInt32, false, 7u32),
            ("id", String, false, "abcd".to_string()),
            ("name", String, false, "Jack".to_string()),
            ("email", String, true, Some("abc@tonbo.io".to_string())),
            ("adress", String, true, None::<String>),
            ("data", Bytes, true, Some(b"hello,tonbo".to_vec())),
            2
        );
        {
            // test project all
            let mut record_ref = record.as_record_ref();
            record_ref.projection(&ProjectionMask::all());
            let columns = record_ref.columns;
            assert_eq!(*cast_arc_value!(columns[0].value, Option<bool>), Some(true));
            assert_eq!(*cast_arc_value!(columns[1].value, Option<u32>), Some(7u32));
            assert_eq!(cast_arc_value!(columns[2].value, String), "abcd");
            assert_eq!(
                *cast_arc_value!(columns[3].value, Option<String>),
                Some("Jack".into()),
            );
            assert_eq!(
                *cast_arc_value!(columns[4].value, Option<String>),
                Some("abc@tonbo.io".into())
            );
            cast_arc_value!(columns[6].value, Option<Vec<u8>>);
            assert_eq!(
                *cast_arc_value!(columns[6].value, Option<Vec<u8>>),
                Some(b"hello,tonbo".to_vec())
            );
        }
        {
            // test project no columns
            let mut record_ref = record.as_record_ref();
            let mask = ProjectionMask::roots(
                &ArrowSchemaConverter::new()
                    .convert(schema.arrow_schema())
                    .unwrap(),
                vec![1],
            );
            record_ref.projection(&mask);
            let columns = record_ref.columns;
            assert_eq!(*cast_arc_value!(columns[0].value, Option<bool>), None);
            assert_eq!(*cast_arc_value!(columns[1].value, Option<u32>), None);
            assert_eq!(cast_arc_value!(columns[2].value, String), "abcd");
            assert_eq!(*cast_arc_value!(columns[3].value, Option<String>), None,);
            assert_eq!(*cast_arc_value!(columns[4].value, Option<String>), None,);
            assert_eq!(*cast_arc_value!(columns[5].value, Option<String>), None,);
            assert_eq!(*cast_arc_value!(columns[6].value, Option<Vec<u8>>), None);
        }
    }
}
