//! The app's behaviour outside a single message: launching, the queue,
//! drives and navigation.

mod init;
mod launch;
pub(crate) use launch::{wowf_first_launch_notice, WOWF_FIRST_LAUNCH_NOTICE};
mod library;
mod nav;
mod queue;
mod splash;
mod subscription;
