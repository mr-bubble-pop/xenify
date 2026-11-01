/// Xenify: a function of one number worked out with floating point (SIN,
/// LN…): `$op` takes and gives `f64`; see `crate::num`.
#[macro_export]
macro_rules! single_number_fn {
    ($fn_name:ident, $op:expr) => {
        $crate::single_num_fn!($fn_name, |n: $crate::num::Num| ($op)(n.f()).map($crate::num::Num::from));
    };
}

/// Xenify: a function of one number on exact numbers (ABS, INT): `$op`
/// takes and gives `Num`.
#[macro_export]
macro_rules! single_num_fn {
    // The macro takes:
    //   1) A function name to define (e.g. fn_sin)
    //   2) The operation to apply (e.g. f64::sin)
    ($fn_name:ident, $op:expr) => {
        pub(crate) fn $fn_name(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
            // 1) Check exactly one argument
            if args.len() != 1 {
                return CalcResult::new_args_number_error(cell);
            }
            // 2) Try to get a "NumberOrArray"
            match self.get_number_or_array(&args[0], cell) {
                // -----------------------------------------
                // Case A: It's a single number
                // -----------------------------------------
                Ok(NumberOrArray::Number(f)) => match $op(f) {
                    Ok(x) => CalcResult::Number(x),
                    Err(Error::DIV) => CalcResult::Error {
                        error: Error::DIV,
                        origin: cell,
                        message: "Divide by 0".to_string(),
                    },
                    Err(Error::VALUE) => CalcResult::Error {
                        error: Error::VALUE,
                        origin: cell,
                        message: "Invalid number".to_string(),
                    },
                    Err(e) => CalcResult::Error {
                        error: e,
                        origin: cell,
                        message: "Unknown error".to_string(),
                    },
                },

                // -----------------------------------------
                // Case B: It's an array, so apply $op
                // element-by-element.
                // -----------------------------------------
                Ok(NumberOrArray::Array(a)) => {
                    let mut array = Vec::new();
                    for row in a {
                        let mut data_row = Vec::with_capacity(row.len());
                        for value in row {
                            match value {
                                // If Boolean, treat as 0.0 or 1.0
                                ArrayNode::Boolean(b) => {
                                    let n = $crate::num::Num::from(if b { 1 } else { 0 });
                                    match $op(n) {
                                        Ok(x) => data_row.push(ArrayNode::Number(x)),
                                        Err(Error::DIV) => {
                                            data_row.push(ArrayNode::Error(Error::DIV))
                                        }
                                        Err(Error::VALUE) => {
                                            data_row.push(ArrayNode::Error(Error::VALUE))
                                        }
                                        Err(e) => data_row.push(ArrayNode::Error(e)),
                                    }
                                }
                                // If Number, apply directly
                                ArrayNode::Number(n) => match $op(n) {
                                    Ok(x) => data_row.push(ArrayNode::Number(x)),
                                    Err(Error::DIV) => data_row.push(ArrayNode::Error(Error::DIV)),
                                    Err(Error::VALUE) => {
                                        data_row.push(ArrayNode::Error(Error::VALUE))
                                    }
                                    Err(e) => data_row.push(ArrayNode::Error(e)),
                                },
                                // If String, parse to f64 then apply or #VALUE! error
                                ArrayNode::String(s) => {
                                    let node = match self.cast_number(&s) {
                                        Some(f) => match $op($crate::num::Num::from(f)) {
                                            Ok(x) => ArrayNode::Number(x),
                                            Err(Error::DIV) => ArrayNode::Error(Error::DIV),
                                            Err(Error::VALUE) => ArrayNode::Error(Error::VALUE),
                                            Err(e) => ArrayNode::Error(e),
                                        },
                                        None => ArrayNode::Error(Error::VALUE),
                                    };
                                    data_row.push(node);
                                }
                                // If Error, propagate the error
                                e @ ArrayNode::Error(_) => {
                                    data_row.push(e);
                                }
                                // Empty cell: treat as 0.0
                                ArrayNode::Empty => match $op($crate::num::Num::ZERO) {
                                    Ok(x) => data_row.push(ArrayNode::Number(x)),
                                    Err(Error::DIV) => data_row.push(ArrayNode::Error(Error::DIV)),
                                    Err(Error::VALUE) => {
                                        data_row.push(ArrayNode::Error(Error::VALUE))
                                    }
                                    Err(e) => data_row.push(ArrayNode::Error(e)),
                                },
                            }
                        }
                        array.push(data_row);
                    }
                    CalcResult::Array(array)
                }

                // -----------------------------------------
                // Case C: It's an Error => just return it
                // -----------------------------------------
                Err(err_result) => err_result,
            }
        }
    };
}
