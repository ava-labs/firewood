#[cfg(feature = "ethhash")]
mod ethhash;
#[cfg(not(feature = "ethhash"))]
mod merkledb;
