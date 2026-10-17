//! Phase-4 RL controller (docs/08): a contextual bandit that owns ONLY the
//! safely-clamped threshold/fusion-weight action space, integrates the two
//! reward routes, and hardens against runaway adaptation.

pub mod arm;
pub mod context;
pub mod error;
pub mod persistence;
pub mod policy;

pub use arm::{LinearThompsonArm, POSTERIOR_SPREAD_0, RIDGE};
pub use context::{
    ActionDim, BucketValue, CONF_BUCKETS, EPOCH_MINUTES, EXPLORATION_END_HOUR, EXPLORATION_EPSILON,
    EXPLORATION_START_HOUR, FEATURE_DIM, FP_WEIGHT_BUCKETS, IOU_BUCKETS, PPO_GAMMA,
    TRACK_BUFFER_BUCKETS, VISION_WEIGHT_BUCKETS, default_action, epoch_of, epoch_of_range,
    feature_vec, in_exploration_window, next_exploration_window,
};
pub use error::RlError;
pub use persistence::SnapshotStore;
pub use policy::{
    BanditPolicy, EpochRecord, IMMEDIATE_WEIGHT, InternalState, MAX_CONF_STEP, REWARD_CLAMP,
    TRACE_HORIZON_STEPS, VETO_FREEZE_THRESHOLD,
};
