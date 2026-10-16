//! Runs a TensorRT engine through the C++ wrapper of `tensorrt.cpp`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;

unsafe extern "C" {
    fn trt_open(path: *const c_char, error: *mut c_char, error_len: usize) -> *mut c_void;
    fn trt_close(handle: *mut c_void);
    fn trt_set_input_shape(
        handle: *mut c_void,
        dims: *const c_int,
        rank: c_int,
        output_dims: *mut c_int,
        output_rank: *mut c_int,
        error: *mut c_char,
        error_len: usize,
    ) -> c_int;
    fn trt_run(
        handle: *mut c_void,
        input: *const f32,
        input_len: usize,
        output: *mut f32,
        output_len: usize,
        error: *mut c_char,
        error_len: usize,
    ) -> c_int;
}

/// An engine file built by `trtexec` from an ONNX model, with one float
/// input and one float output.
pub struct Engine {
    handle: *mut c_void,
}

// The execution context can move to another thread; `&mut self` keeps it
// used by one at a time.
unsafe impl Send for Engine {}

impl Engine {
    pub fn open(path: &Path) -> Result<Self, String> {
        let c_path =
            CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
        let mut error = [0 as c_char; 256];
        let handle = unsafe { trt_open(c_path.as_ptr(), error.as_mut_ptr(), error.len()) };
        if handle.is_null() {
            return Err(message(&error));
        }
        Ok(Self { handle })
    }

    /// Runs the model on `input`, of shape `dims`, and returns its output
    /// with the output's shape.
    pub fn run(&mut self, input: &[f32], dims: &[usize]) -> Result<(Vec<f32>, Vec<usize>), String> {
        let dims: Vec<c_int> = dims.iter().map(|&d| d as c_int).collect();
        let mut output_dims = [0 as c_int; 8];
        let mut output_rank: c_int = 0;
        let mut error = [0 as c_char; 256];
        let status = unsafe {
            trt_set_input_shape(
                self.handle,
                dims.as_ptr(),
                dims.len() as c_int,
                output_dims.as_mut_ptr(),
                &mut output_rank,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if status != 0 {
            return Err(message(&error));
        }
        let shape: Vec<usize> = output_dims[..output_rank as usize]
            .iter()
            .map(|&d| d.max(0) as usize)
            .collect();
        let mut output = vec![0.0; shape.iter().product()];
        let status = unsafe {
            trt_run(
                self.handle,
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if status != 0 {
            return Err(message(&error));
        }
        Ok((output, shape))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { trt_close(self.handle) }
    }
}

fn message(error: &[c_char]) -> String {
    unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}
