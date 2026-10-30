pub struct Handle;

impl Handle {
    pub fn into_raw(self) -> *const u8 {
        c"capsule".as_ptr().cast()
    }
}

pub fn get_handle() -> Result<Handle, std::io::Error> {
    Ok(Handle)
}
