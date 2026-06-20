use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, OnceLock};

pub const DEFAULT_MIN_REGION_SIZE: i32 = 64;
pub const DEFAULT_SPLIT_RATIO: SplitRatio = SplitRatio(0.5);
const EMPTY_WORKSPACE_NEXT_TAB_ID: u64 = 0;
const EMPTY_WORKSPACE_NEXT_REGION_ID: u64 = 0;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, Clone, PartialEq)]
pub enum DomainError {
    EmptyTabName,
    EmptyTabPresetName,
    EmptyProgramExecutablePath,
    InvalidProgramArgument,
    NoActiveTab,
    TabPresetNotFound(String),
    TabPresetTargetHasPlacements(TabId),
    TabNotFound(TabId),
    DuplicateTab(TabId),
    RegionNotFound(RegionId),
    DuplicateRegion(RegionId),
    PlacementTabMismatch {
        expected: TabId,
        actual: TabId,
    },
    PlacementNotFound {
        tab_id: TabId,
        region_id: RegionId,
    },
    RegionAlreadyOccupied(RegionId),
    WindowAlreadyPlaced(WindowHandle),
    InvalidWindowHandle,
    WindowSnapshotMismatch {
        placement: WindowHandle,
        snapshot: WindowHandle,
    },
    InvalidRect {
        width: i32,
        height: i32,
    },
    InvalidSplitRatio(f64),
    InvalidSplitPosition {
        direction: SplitDirection,
        available: i32,
        first_child: i32,
        min_region_size: i32,
    },
    InvalidMinimumRegionSize(i32),
    RegionTooSmall {
        direction: SplitDirection,
        available: i32,
        min_required: i32,
    },
    SplitterNotFound,
    RootRegionCannotBeDeleted(RegionId),
    LayoutDepthExceeded {
        max_depth: usize,
    },
    CoordinateOverflow,
    IdExhausted(&'static str),
}

impl DomainError {
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::EmptyTabName => "탭 이름을 비워 둘 수 없습니다.",
            Self::EmptyTabPresetName => "탭 preset 이름을 비워 둘 수 없습니다.",
            Self::EmptyProgramExecutablePath => "프로그램 실행 파일 경로를 비워 둘 수 없습니다.",
            Self::InvalidProgramArgument => {
                "프로그램 실행 argument에 유효하지 않은 문자가 있습니다."
            }
            Self::NoActiveTab => "활성 탭이 없습니다.",
            Self::TabPresetNotFound(_) => "요청한 탭 preset을 찾을 수 없습니다.",
            Self::TabPresetTargetHasPlacements(_) => {
                "배치된 외부 윈도우가 있는 탭에는 탭 preset을 적용할 수 없습니다."
            }
            Self::TabNotFound(_) => "요청한 탭을 찾을 수 없습니다.",
            Self::DuplicateTab(_) => "이미 존재하는 탭 ID입니다.",
            Self::RegionNotFound(_) => "요청한 영역을 찾을 수 없습니다.",
            Self::DuplicateRegion(_) => "이미 존재하는 영역 ID입니다.",
            Self::PlacementTabMismatch { .. } => "배치 정보의 탭이 대상 탭과 일치하지 않습니다.",
            Self::PlacementNotFound { .. } => "요청한 영역에 배치된 외부 윈도우가 없습니다.",
            Self::RegionAlreadyOccupied(_) => "해당 영역에는 이미 외부 윈도우가 배치되어 있습니다.",
            Self::WindowAlreadyPlaced(_) => "해당 외부 윈도우는 이미 배치되어 있습니다.",
            Self::InvalidWindowHandle => "유효하지 않은 외부 윈도우입니다.",
            Self::WindowSnapshotMismatch { .. } => {
                "배치 전 윈도우 상태와 대상 윈도우가 일치하지 않습니다."
            }
            Self::InvalidRect { .. } => "유효하지 않은 영역 좌표입니다.",
            Self::InvalidSplitRatio(_) => "유효하지 않은 splitter 비율입니다.",
            Self::InvalidSplitPosition { .. } => "splitter 위치에서 비율을 계산할 수 없습니다.",
            Self::InvalidMinimumRegionSize(_) => "유효하지 않은 최소 영역 크기입니다.",
            Self::RegionTooSmall { .. } => "영역이 너무 작아 splitter를 계산할 수 없습니다.",
            Self::SplitterNotFound => "요청한 splitter를 찾을 수 없습니다.",
            Self::RootRegionCannotBeDeleted(_) => "root 영역은 삭제할 수 없습니다.",
            Self::LayoutDepthExceeded { .. } => "설정 layout 중첩이 너무 깊습니다.",
            Self::CoordinateOverflow => "영역 좌표 계산 중 범위를 초과했습니다.",
            Self::IdExhausted(_) => "새 ID를 생성할 수 없습니다.",
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTabName => write!(formatter, "tab name is empty"),
            Self::EmptyTabPresetName => write!(formatter, "tab preset name is empty"),
            Self::EmptyProgramExecutablePath => {
                write!(formatter, "program executable path is empty")
            }
            Self::InvalidProgramArgument => write!(formatter, "program argument is invalid"),
            Self::NoActiveTab => write!(formatter, "no active tab"),
            Self::TabPresetNotFound(name) => {
                write!(formatter, "tab preset not found: {name}")
            }
            Self::TabPresetTargetHasPlacements(tab_id) => write!(
                formatter,
                "tab preset target tab has placements: tab={tab_id:?}"
            ),
            Self::TabNotFound(tab_id) => write!(formatter, "tab not found: {tab_id:?}"),
            Self::DuplicateTab(tab_id) => write!(formatter, "duplicate tab id: {tab_id:?}"),
            Self::RegionNotFound(region_id) => write!(formatter, "region not found: {region_id:?}"),
            Self::DuplicateRegion(region_id) => {
                write!(formatter, "duplicate region id: {region_id:?}")
            }
            Self::PlacementTabMismatch { expected, actual } => write!(
                formatter,
                "placement tab mismatch: expected {expected:?}, actual {actual:?}"
            ),
            Self::PlacementNotFound { tab_id, region_id } => write!(
                formatter,
                "placement not found: tab={tab_id:?}, region={region_id:?}"
            ),
            Self::RegionAlreadyOccupied(region_id) => {
                write!(formatter, "region already occupied: {region_id:?}")
            }
            Self::WindowAlreadyPlaced(hwnd) => {
                write!(formatter, "window already placed: {hwnd:?}")
            }
            Self::InvalidWindowHandle => write!(formatter, "invalid window handle"),
            Self::WindowSnapshotMismatch {
                placement,
                snapshot,
            } => write!(
                formatter,
                "window snapshot mismatch: placement {placement:?}, snapshot {snapshot:?}"
            ),
            Self::InvalidRect { width, height } => {
                write!(
                    formatter,
                    "invalid rect size: width={width}, height={height}"
                )
            }
            Self::InvalidSplitRatio(ratio) => write!(formatter, "invalid split ratio: {ratio}"),
            Self::InvalidSplitPosition {
                direction,
                available,
                first_child,
                min_region_size,
            } => write!(
                formatter,
                "invalid split position for {direction:?}: available={available}, first_child={first_child}, min_region_size={min_region_size}"
            ),
            Self::InvalidMinimumRegionSize(size) => {
                write!(formatter, "invalid minimum region size: {size}")
            }
            Self::RegionTooSmall {
                direction,
                available,
                min_required,
            } => write!(
                formatter,
                "region too small for {direction:?} split: available={available}, min_required={min_required}"
            ),
            Self::SplitterNotFound => write!(formatter, "splitter not found"),
            Self::RootRegionCannotBeDeleted(region_id) => {
                write!(formatter, "root region cannot be deleted: {region_id:?}")
            }
            Self::LayoutDepthExceeded { max_depth } => {
                write!(
                    formatter,
                    "layout split depth exceeds maximum: max_depth={max_depth}"
                )
            }
            Self::CoordinateOverflow => write!(formatter, "coordinate overflow"),
            Self::IdExhausted(scope) => write!(formatter, "id exhausted: {scope}"),
        }
    }
}

impl Error for DomainError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(u64);

impl TabId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(u64);

impl RegionId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowHandle(isize);

impl WindowHandle {
    pub fn new(raw: isize) -> DomainResult<Self> {
        if raw == 0 {
            Err(DomainError::InvalidWindowHandle)
        } else {
            Ok(Self(raw))
        }
    }

    pub const fn raw(self) -> isize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowIdentity {
    thread_id: u32,
    process_id: u32,
    validation_token: Option<usize>,
}

impl WindowIdentity {
    pub const fn new(thread_id: u32, process_id: u32) -> Self {
        Self {
            thread_id,
            process_id,
            validation_token: None,
        }
    }

    pub const fn thread_id(self) -> u32 {
        self.thread_id
    }

    pub const fn process_id(self) -> u32 {
        self.process_id
    }

    pub const fn validation_token(self) -> Option<usize> {
        self.validation_token
    }

    pub const fn with_validation_token(mut self, validation_token: usize) -> Self {
        self.validation_token = Some(validation_token);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitRatio(f64);

impl SplitRatio {
    pub fn new(value: f64) -> DomainResult<Self> {
        if value.is_finite() && value > 0.0 && value < 1.0 {
            Ok(Self(value))
        } else {
            Err(DomainError::InvalidSplitRatio(value))
        }
    }

    pub fn from_first_child_size(
        direction: SplitDirection,
        available: i32,
        first_child: i32,
        min_region_size: i32,
    ) -> DomainResult<Self> {
        if min_region_size <= 0 {
            return Err(DomainError::InvalidMinimumRegionSize(min_region_size));
        }

        let min_required = min_region_size
            .checked_mul(2)
            .ok_or(DomainError::CoordinateOverflow)?;

        if available < min_required {
            return Err(DomainError::RegionTooSmall {
                direction,
                available,
                min_required,
            });
        }

        let second_child =
            available
                .checked_sub(first_child)
                .ok_or(DomainError::InvalidSplitPosition {
                    direction,
                    available,
                    first_child,
                    min_region_size,
                })?;

        if first_child < min_region_size || second_child < min_region_size {
            return Err(DomainError::InvalidSplitPosition {
                direction,
                available,
                first_child,
                min_region_size,
            });
        }

        Self::new(f64::from(first_child) / f64::from(available))
    }

    pub const fn value(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitterChild {
    First,
    Second,
}

pub struct SplitterPath {
    inner: SplitterPathInner,
}

enum SplitterPathInner {
    Root,
    Flat(Arc<[SplitterChild]>),
    Linked {
        tail: Arc<SplitterPathNode>,
        steps: OnceLock<Vec<SplitterChild>>,
    },
}

struct SplitterPathNode {
    parent: Option<Arc<SplitterPathNode>>,
    child: SplitterChild,
    len: usize,
}

impl fmt::Debug for SplitterPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SplitterPath")
            .field("steps", &self.steps())
            .finish()
    }
}

impl Clone for SplitterPath {
    fn clone(&self) -> Self {
        let inner = match &self.inner {
            SplitterPathInner::Root => SplitterPathInner::Root,
            SplitterPathInner::Flat(steps) => SplitterPathInner::Flat(Arc::clone(steps)),
            SplitterPathInner::Linked { tail, .. } => SplitterPathInner::Linked {
                tail: Arc::clone(tail),
                steps: OnceLock::new(),
            },
        };
        Self { inner }
    }
}

impl PartialEq for SplitterPath {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.steps() == other.steps()
    }
}

impl Eq for SplitterPath {}

impl SplitterPathNode {
    fn from_steps(steps: &[SplitterChild]) -> Option<Arc<Self>> {
        let mut tail = None;
        for (index, child) in steps.iter().copied().enumerate() {
            tail = Some(Arc::new(Self {
                parent: tail,
                child,
                len: index + 1,
            }));
        }
        tail
    }

    fn collect_steps(tail: &Self) -> Vec<SplitterChild> {
        let mut steps = Vec::with_capacity(tail.len);
        let mut node = Some(tail);
        while let Some(current) = node {
            steps.push(current.child);
            node = current.parent.as_deref();
        }
        steps.reverse();
        steps
    }
}

impl SplitterPath {
    pub fn root() -> Self {
        Self {
            inner: SplitterPathInner::Root,
        }
    }

    pub fn child(&self, child: SplitterChild) -> Self {
        let parent = self.linked_tail();
        let len = parent.as_ref().map_or(0, |tail| tail.len) + 1;
        Self {
            inner: SplitterPathInner::Linked {
                tail: Arc::new(SplitterPathNode { parent, child, len }),
                steps: OnceLock::new(),
            },
        }
    }

    pub fn steps(&self) -> &[SplitterChild] {
        match &self.inner {
            SplitterPathInner::Root => &[],
            SplitterPathInner::Flat(steps) => steps.as_ref(),
            SplitterPathInner::Linked { tail, steps } => steps
                .get_or_init(|| SplitterPathNode::collect_steps(tail.as_ref()))
                .as_slice(),
        }
    }

    fn from_steps(steps: &[SplitterChild]) -> Self {
        if steps.is_empty() {
            Self::root()
        } else {
            Self {
                inner: SplitterPathInner::Flat(Arc::from(steps)),
            }
        }
    }

    fn len(&self) -> usize {
        match &self.inner {
            SplitterPathInner::Root => 0,
            SplitterPathInner::Flat(steps) => steps.len(),
            SplitterPathInner::Linked { tail, .. } => tail.len,
        }
    }

    fn linked_tail(&self) -> Option<Arc<SplitterPathNode>> {
        match &self.inner {
            SplitterPathInner::Root => None,
            SplitterPathInner::Flat(steps) => SplitterPathNode::from_steps(steps.as_ref()),
            SplitterPathInner::Linked { tail, .. } => Some(Arc::clone(tail)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitterRect {
    path: SplitterPath,
    direction: SplitDirection,
    rect: Rect,
}

impl SplitterRect {
    pub fn new(path: SplitterPath, direction: SplitDirection, rect: Rect) -> Self {
        Self {
            path,
            direction,
            rect,
        }
    }

    pub const fn path(&self) -> &SplitterPath {
        &self.path
    }

    pub const fn direction(&self) -> SplitDirection {
        self.direction
    }

    pub const fn rect(&self) -> Rect {
        self.rect
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SplitterResizeRollback {
    previous_ratio: SplitRatio,
}

impl SplitterResizeRollback {
    const fn new(previous_ratio: SplitRatio) -> Self {
        Self { previous_ratio }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> DomainResult<Self> {
        if width <= 0 || height <= 0 {
            return Err(DomainError::InvalidRect { width, height });
        }

        let rect = Self {
            left,
            top,
            width,
            height,
        };
        rect.checked_right()?;
        rect.checked_bottom()?;

        Ok(rect)
    }

    pub const fn left(self) -> i32 {
        self.left
    }

    pub const fn top(self) -> i32 {
        self.top
    }

    pub const fn width(self) -> i32 {
        self.width
    }

    pub const fn height(self) -> i32 {
        self.height
    }

    pub fn contains_point(self, x: i32, y: i32) -> bool {
        let left = i64::from(self.left);
        let top = i64::from(self.top);
        let right = left + i64::from(self.width);
        let bottom = top + i64::from(self.height);
        let x = i64::from(x);
        let y = i64::from(y);

        x >= left && x < right && y >= top && y < bottom
    }

    pub fn translated(self, dx: i32, dy: i32) -> DomainResult<Self> {
        let left = self
            .left
            .checked_add(dx)
            .ok_or(DomainError::CoordinateOverflow)?;
        let top = self
            .top
            .checked_add(dy)
            .ok_or(DomainError::CoordinateOverflow)?;
        Self::new(left, top, self.width, self.height)
    }

    fn checked_right(self) -> DomainResult<i32> {
        self.left
            .checked_add(self.width)
            .ok_or(DomainError::CoordinateOverflow)
    }

    fn checked_bottom(self) -> DomainResult<i32> {
        self.top
            .checked_add(self.height)
            .ok_or(DomainError::CoordinateOverflow)
    }

    fn split(
        self,
        direction: SplitDirection,
        ratio: SplitRatio,
        min_region_size: i32,
    ) -> DomainResult<(Self, Self)> {
        if min_region_size <= 0 {
            return Err(DomainError::InvalidMinimumRegionSize(min_region_size));
        }

        match direction {
            SplitDirection::Vertical => self.split_axis(direction, self.width, min_region_size),
            SplitDirection::Horizontal => self.split_axis(direction, self.height, min_region_size),
        }
        .and_then(|available| self.rects_for_split(direction, available, ratio, min_region_size))
    }

    fn split_axis(
        self,
        direction: SplitDirection,
        available: i32,
        min_region_size: i32,
    ) -> DomainResult<i32> {
        let min_required = min_region_size
            .checked_mul(2)
            .ok_or(DomainError::CoordinateOverflow)?;

        if available < min_required {
            return Err(DomainError::RegionTooSmall {
                direction,
                available,
                min_required,
            });
        }

        Ok(available)
    }

    fn rects_for_split(
        self,
        direction: SplitDirection,
        available: i32,
        ratio: SplitRatio,
        min_region_size: i32,
    ) -> DomainResult<(Self, Self)> {
        let split_at = scaled_split_position(available, ratio, min_region_size);

        match direction {
            SplitDirection::Vertical => {
                let second_left = self
                    .left
                    .checked_add(split_at)
                    .ok_or(DomainError::CoordinateOverflow)?;
                Ok((
                    Self::new(self.left, self.top, split_at, self.height)?,
                    Self::new(second_left, self.top, available - split_at, self.height)?,
                ))
            }
            SplitDirection::Horizontal => {
                let second_top = self
                    .top
                    .checked_add(split_at)
                    .ok_or(DomainError::CoordinateOverflow)?;
                Ok((
                    Self::new(self.left, self.top, self.width, split_at)?,
                    Self::new(self.left, second_top, self.width, available - split_at)?,
                ))
            }
        }
    }
}

fn scaled_split_position(available: i32, ratio: SplitRatio, min_region_size: i32) -> i32 {
    let proposed = (f64::from(available) * ratio.value()).round() as i32;
    proposed.clamp(min_region_size, available - min_region_size)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionRect {
    region_id: RegionId,
    rect: Rect,
}

impl RegionRect {
    pub const fn new(region_id: RegionId, rect: Rect) -> Self {
        Self { region_id, rect }
    }

    pub const fn region_id(self) -> RegionId {
        self.region_id
    }

    pub const fn rect(self) -> Rect {
        self.rect
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutNode {
    Region {
        id: RegionId,
    },
    Split {
        direction: SplitDirection,
        ratio: SplitRatio,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    pub const fn single_region(id: RegionId) -> Self {
        Self::Region { id }
    }

    pub fn region_rects(
        &self,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        let mut rects = Vec::new();
        LayoutGeometry::collect_region_rects(self, bounds, min_region_size, &mut rects)?;
        Ok(rects)
    }

    pub fn find_region_rect(
        &self,
        target: RegionId,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Rect> {
        LayoutGeometry::find_region_rect_inner(self, target, bounds, min_region_size)?
            .ok_or(DomainError::RegionNotFound(target))
    }

    pub fn region_and_splitter_rects(
        &self,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<(Vec<RegionRect>, Vec<SplitterRect>)> {
        let mut regions = Vec::new();
        let mut splitters = Vec::new();
        self.region_and_splitter_rects_into(
            bounds,
            tolerance,
            min_region_size,
            &mut regions,
            &mut splitters,
        )?;
        Ok((regions, splitters))
    }

    pub fn region_and_splitter_rects_into(
        &self,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
        regions: &mut Vec<RegionRect>,
        splitters: &mut Vec<SplitterRect>,
    ) -> DomainResult<()> {
        regions.clear();
        splitters.clear();

        let path = SplitterPath::root();
        let result = LayoutGeometry::collect_region_and_splitter_rects(
            self,
            bounds,
            min_region_size,
            tolerance.max(0),
            &path,
            regions,
            splitters,
        );
        if result.is_err() {
            regions.clear();
            splitters.clear();
        }
        result
    }

    pub fn contains_region(&self, target: RegionId) -> bool {
        match self {
            Self::Region { id } => *id == target,
            Self::Split { first, second, .. } => {
                first.contains_region(target) || second.contains_region(target)
            }
        }
    }

    pub fn region_ids(&self) -> DomainResult<Vec<RegionId>> {
        let mut region_ids = Vec::new();
        let mut seen = HashSet::new();
        self.visit_unique_region_ids(&mut seen, &mut |id| region_ids.push(id))?;
        Ok(region_ids)
    }

    pub fn split_region(
        &mut self,
        target: RegionId,
        direction: SplitDirection,
        new_region: RegionId,
        ratio: SplitRatio,
    ) -> DomainResult<()> {
        if target == new_region || self.contains_region(new_region) {
            return Err(DomainError::DuplicateRegion(new_region));
        }

        if self.split_region_inner(target, direction, new_region, ratio)? {
            Ok(())
        } else {
            Err(DomainError::RegionNotFound(target))
        }
    }

    pub fn delete_region(&mut self, target: RegionId) -> DomainResult<()> {
        match self {
            Self::Region { id } if *id == target => {
                Err(DomainError::RootRegionCannotBeDeleted(target))
            }
            Self::Region { .. } => Err(DomainError::RegionNotFound(target)),
            Self::Split { .. } => {
                if self.delete_region_inner(target)? {
                    Ok(())
                } else {
                    Err(DomainError::RegionNotFound(target))
                }
            }
        }
    }

    pub fn hit_test(
        &self,
        bounds: Rect,
        x: i32,
        y: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<RegionId>> {
        LayoutGeometry::hit_test_region_inner(self, bounds, x, y, min_region_size)
    }

    pub fn splitter_rects(
        &self,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<Vec<SplitterRect>> {
        let mut splitters = Vec::new();
        self.splitter_rects_into(bounds, tolerance, min_region_size, &mut splitters)?;
        Ok(splitters)
    }

    pub fn splitter_rects_into(
        &self,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
        splitters: &mut Vec<SplitterRect>,
    ) -> DomainResult<()> {
        splitters.clear();

        let path = SplitterPath::root();
        let result = LayoutGeometry::collect_splitter_rects(
            self,
            bounds,
            min_region_size,
            tolerance.max(0),
            &path,
            splitters,
        );
        if result.is_err() {
            splitters.clear();
        }
        result
    }

    pub fn hit_test_splitter(
        &self,
        bounds: Rect,
        x: i32,
        y: i32,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<SplitterRect>> {
        let mut path = Vec::new();
        LayoutGeometry::hit_test_splitter_inner(
            self,
            bounds,
            x,
            y,
            tolerance.max(0),
            min_region_size,
            &mut path,
        )
    }

    pub fn resize_splitter(
        &mut self,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<()> {
        self.resize_splitter_inner(path.steps(), bounds, pointer_x, pointer_y, min_region_size)
    }

    pub(crate) fn resize_splitter_if_changed(
        &mut self,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<SplitterResizeRollback>> {
        self.resize_splitter_if_changed_inner(
            path.steps(),
            bounds,
            pointer_x,
            pointer_y,
            min_region_size,
        )
    }

    pub(crate) fn rollback_splitter_resize(
        &mut self,
        path: &SplitterPath,
        rollback: SplitterResizeRollback,
    ) -> DomainResult<()> {
        self.set_splitter_ratio_inner(path.steps(), rollback.previous_ratio)
    }

    pub(crate) fn region_rects_before_splitter_resize(
        &self,
        path: &SplitterPath,
        bounds: Rect,
        rollback: SplitterResizeRollback,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        let mut rects = Vec::new();
        LayoutGeometry::collect_region_rects_with_splitter_ratio(
            self,
            path.steps(),
            bounds,
            rollback.previous_ratio,
            min_region_size,
            &mut rects,
        )?;
        Ok(rects)
    }

    #[cfg(any(test, target_os = "windows"))]
    pub(crate) fn region_rects_for_splitter_resize(
        &self,
        path: &SplitterPath,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        let mut rects = Vec::new();
        LayoutGeometry::collect_region_rects_for_splitter(
            self,
            path.steps(),
            bounds,
            min_region_size,
            &mut rects,
        )?;
        Ok(rects)
    }

    pub fn resize_splitter_would_change(
        &self,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<bool> {
        self.resize_splitter_would_change_inner(
            path.steps(),
            bounds,
            pointer_x,
            pointer_y,
            min_region_size,
        )
    }
}

struct LayoutGeometry;

impl LayoutGeometry {
    fn collect_region_rects(
        node: &LayoutNode,
        bounds: Rect,
        min_region_size: i32,
        rects: &mut Vec<RegionRect>,
    ) -> DomainResult<()> {
        match node {
            LayoutNode::Region { id } => {
                rects.push(RegionRect::new(*id, bounds));
                Ok(())
            }
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;
                Self::collect_region_rects(first, first_rect, min_region_size, rects)?;
                Self::collect_region_rects(second, second_rect, min_region_size, rects)
            }
        }
    }

    fn find_region_rect_inner(
        node: &LayoutNode,
        target: RegionId,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Option<Rect>> {
        match node {
            LayoutNode::Region { id } => Ok((*id == target).then_some(bounds)),
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;

                if let Some(rect) =
                    Self::find_region_rect_inner(first, target, first_rect, min_region_size)?
                {
                    return Ok(Some(rect));
                }

                Self::find_region_rect_inner(second, target, second_rect, min_region_size)
            }
        }
    }

    fn collect_region_rects_with_splitter_ratio(
        node: &LayoutNode,
        path: &[SplitterChild],
        bounds: Rect,
        replacement_ratio: SplitRatio,
        min_region_size: i32,
        rects: &mut Vec<RegionRect>,
    ) -> DomainResult<()> {
        match node {
            LayoutNode::Region { .. } => Err(DomainError::SplitterNotFound),
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let active_ratio = if path.is_empty() {
                    replacement_ratio
                } else {
                    *ratio
                };
                let (first_rect, second_rect) =
                    bounds.split(*direction, active_ratio, min_region_size)?;

                if path.is_empty() {
                    Self::collect_region_rects(first, first_rect, min_region_size, rects)?;
                    return Self::collect_region_rects(second, second_rect, min_region_size, rects);
                }

                match path[0] {
                    SplitterChild::First => {
                        Self::collect_region_rects_with_splitter_ratio(
                            first,
                            &path[1..],
                            first_rect,
                            replacement_ratio,
                            min_region_size,
                            rects,
                        )?;
                        Self::collect_region_rects(second, second_rect, min_region_size, rects)
                    }
                    SplitterChild::Second => {
                        Self::collect_region_rects(first, first_rect, min_region_size, rects)?;
                        Self::collect_region_rects_with_splitter_ratio(
                            second,
                            &path[1..],
                            second_rect,
                            replacement_ratio,
                            min_region_size,
                            rects,
                        )
                    }
                }
            }
        }
    }

    #[cfg(any(test, target_os = "windows"))]
    fn collect_region_rects_for_splitter(
        node: &LayoutNode,
        path: &[SplitterChild],
        bounds: Rect,
        min_region_size: i32,
        rects: &mut Vec<RegionRect>,
    ) -> DomainResult<()> {
        match node {
            LayoutNode::Region { .. } => Err(DomainError::SplitterNotFound),
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;

                if path.is_empty() {
                    Self::collect_region_rects(first, first_rect, min_region_size, rects)?;
                    return Self::collect_region_rects(second, second_rect, min_region_size, rects);
                }

                match path[0] {
                    SplitterChild::First => Self::collect_region_rects_for_splitter(
                        first,
                        &path[1..],
                        first_rect,
                        min_region_size,
                        rects,
                    ),
                    SplitterChild::Second => Self::collect_region_rects_for_splitter(
                        second,
                        &path[1..],
                        second_rect,
                        min_region_size,
                        rects,
                    ),
                }
            }
        }
    }

    fn hit_test_region_inner(
        node: &LayoutNode,
        bounds: Rect,
        x: i32,
        y: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<RegionId>> {
        match node {
            LayoutNode::Region { id } => Ok(bounds.contains_point(x, y).then_some(*id)),
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;

                if first_rect.contains_point(x, y) {
                    return Self::hit_test_region_inner(first, first_rect, x, y, min_region_size);
                }
                if second_rect.contains_point(x, y) {
                    return Self::hit_test_region_inner(second, second_rect, x, y, min_region_size);
                }

                Ok(None)
            }
        }
    }

    fn collect_region_and_splitter_rects(
        node: &LayoutNode,
        bounds: Rect,
        min_region_size: i32,
        tolerance: i32,
        path: &SplitterPath,
        regions: &mut Vec<RegionRect>,
        splitters: &mut Vec<SplitterRect>,
    ) -> DomainResult<()> {
        match node {
            LayoutNode::Region { id } => {
                regions.push(RegionRect::new(*id, bounds));
                Ok(())
            }
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;
                splitters.push(SplitterRect::new(
                    (*path).clone(),
                    *direction,
                    splitter_hit_rect(*direction, first_rect, bounds, tolerance)?,
                ));
                let first_result = match first.as_ref() {
                    LayoutNode::Region { .. } => Self::collect_region_and_splitter_rects(
                        first,
                        first_rect,
                        min_region_size,
                        tolerance,
                        path,
                        regions,
                        splitters,
                    ),
                    LayoutNode::Split { .. } => {
                        let first_path = path.child(SplitterChild::First);
                        Self::collect_region_and_splitter_rects(
                            first,
                            first_rect,
                            min_region_size,
                            tolerance,
                            &first_path,
                            regions,
                            splitters,
                        )
                    }
                };
                first_result?;

                match second.as_ref() {
                    LayoutNode::Region { .. } => Self::collect_region_and_splitter_rects(
                        second,
                        second_rect,
                        min_region_size,
                        tolerance,
                        path,
                        regions,
                        splitters,
                    ),
                    LayoutNode::Split { .. } => {
                        let second_path = path.child(SplitterChild::Second);
                        Self::collect_region_and_splitter_rects(
                            second,
                            second_rect,
                            min_region_size,
                            tolerance,
                            &second_path,
                            regions,
                            splitters,
                        )
                    }
                }
            }
        }
    }

    fn hit_test_splitter_inner(
        node: &LayoutNode,
        bounds: Rect,
        x: i32,
        y: i32,
        tolerance: i32,
        min_region_size: i32,
        path: &mut Vec<SplitterChild>,
    ) -> DomainResult<Option<SplitterRect>> {
        match node {
            LayoutNode::Region { .. } => Ok(None),
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;
                let rect = splitter_hit_rect(*direction, first_rect, bounds, tolerance)?;
                if rect.contains_point(x, y) {
                    return Ok(Some(SplitterRect::new(
                        SplitterPath::from_steps(path),
                        *direction,
                        rect,
                    )));
                }

                let next = if first_rect.contains_point(x, y) {
                    Some((SplitterChild::First, first.as_ref(), first_rect))
                } else if second_rect.contains_point(x, y) {
                    Some((SplitterChild::Second, second.as_ref(), second_rect))
                } else {
                    None
                };

                let Some((step, child, child_bounds)) = next else {
                    return Ok(None);
                };

                path.push(step);
                let result = Self::hit_test_splitter_inner(
                    child,
                    child_bounds,
                    x,
                    y,
                    tolerance,
                    min_region_size,
                    path,
                );
                path.pop();
                result
            }
        }
    }
}

impl LayoutNode {
    fn visit_unique_region_ids(
        &self,
        seen: &mut HashSet<RegionId>,
        on_region: &mut impl FnMut(RegionId),
    ) -> DomainResult<()> {
        match self {
            Self::Region { id } => {
                if seen.insert(*id) {
                    on_region(*id);
                    Ok(())
                } else {
                    Err(DomainError::DuplicateRegion(*id))
                }
            }
            Self::Split { first, second, .. } => {
                first.visit_unique_region_ids(seen, on_region)?;
                second.visit_unique_region_ids(seen, on_region)
            }
        }
    }

    #[cfg(test)]
    fn max_region_id(&self) -> DomainResult<Option<u64>> {
        let mut seen = HashSet::new();
        let mut max_region_id: Option<u64> = None;
        self.visit_unique_region_ids(&mut seen, &mut |id| {
            let value = id.value();
            max_region_id = Some(match max_region_id {
                Some(max) => max.max(value),
                None => value,
            });
        })?;
        Ok(max_region_id)
    }
}

impl LayoutGeometry {
    fn collect_splitter_rects(
        node: &LayoutNode,
        bounds: Rect,
        min_region_size: i32,
        tolerance: i32,
        path: &SplitterPath,
        splitters: &mut Vec<SplitterRect>,
    ) -> DomainResult<()> {
        match node {
            LayoutNode::Region { .. } => Ok(()),
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;
                splitters.push(SplitterRect::new(
                    (*path).clone(),
                    *direction,
                    splitter_hit_rect(*direction, first_rect, bounds, tolerance)?,
                ));
                let first_result = match first.as_ref() {
                    LayoutNode::Region { .. } => Self::collect_splitter_rects(
                        first,
                        first_rect,
                        min_region_size,
                        tolerance,
                        path,
                        splitters,
                    ),
                    LayoutNode::Split { .. } => {
                        let first_path = path.child(SplitterChild::First);
                        Self::collect_splitter_rects(
                            first,
                            first_rect,
                            min_region_size,
                            tolerance,
                            &first_path,
                            splitters,
                        )
                    }
                };
                first_result?;

                match second.as_ref() {
                    LayoutNode::Region { .. } => Self::collect_splitter_rects(
                        second,
                        second_rect,
                        min_region_size,
                        tolerance,
                        path,
                        splitters,
                    ),
                    LayoutNode::Split { .. } => {
                        let second_path = path.child(SplitterChild::Second);
                        Self::collect_splitter_rects(
                            second,
                            second_rect,
                            min_region_size,
                            tolerance,
                            &second_path,
                            splitters,
                        )
                    }
                }
            }
        }
    }
}

impl LayoutNode {
    fn is_region(&self, target: RegionId) -> bool {
        matches!(self, Self::Region { id } if *id == target)
    }

    fn split_region_inner(
        &mut self,
        target: RegionId,
        direction: SplitDirection,
        new_region: RegionId,
        ratio: SplitRatio,
    ) -> DomainResult<bool> {
        match self {
            Self::Region { id } if *id == target => {
                *self = Self::Split {
                    direction,
                    ratio,
                    first: Box::new(Self::Region { id: target }),
                    second: Box::new(Self::Region { id: new_region }),
                };
                Ok(true)
            }
            Self::Region { .. } => Ok(false),
            Self::Split { first, second, .. } => {
                if first.split_region_inner(target, direction, new_region, ratio)? {
                    Ok(true)
                } else {
                    second.split_region_inner(target, direction, new_region, ratio)
                }
            }
        }
    }

    fn delete_region_inner(&mut self, target: RegionId) -> DomainResult<bool> {
        match self {
            Self::Region { .. } => Ok(false),
            Self::Split { first, second, .. } => {
                let delete_first = first.is_region(target);
                let delete_second = second.is_region(target);

                if delete_first {
                    let replacement =
                        std::mem::replace(second, Box::new(Self::Region { id: target }));
                    *self = *replacement;
                    return Ok(true);
                }

                if delete_second {
                    let replacement =
                        std::mem::replace(first, Box::new(Self::Region { id: target }));
                    *self = *replacement;
                    return Ok(true);
                }

                if first.delete_region_inner(target)? {
                    Ok(true)
                } else {
                    second.delete_region_inner(target)
                }
            }
        }
    }

    fn resize_splitter_inner(
        &mut self,
        path: &[SplitterChild],
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<()> {
        self.resize_splitter_if_changed_inner(path, bounds, pointer_x, pointer_y, min_region_size)
            .map(|_| ())
    }

    fn resize_splitter_if_changed_inner(
        &mut self,
        path: &[SplitterChild],
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<SplitterResizeRollback>> {
        match self {
            Self::Region { .. } => Err(DomainError::SplitterNotFound),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if path.is_empty() {
                    let next_ratio = split_ratio_from_pointer(
                        *direction,
                        bounds,
                        pointer_x,
                        pointer_y,
                        min_region_size,
                    )?;
                    if next_ratio == *ratio {
                        return Ok(None);
                    }

                    let rollback = SplitterResizeRollback::new(*ratio);
                    *ratio = next_ratio;
                    return Ok(Some(rollback));
                }

                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;
                match path[0] {
                    SplitterChild::First => first.resize_splitter_if_changed_inner(
                        &path[1..],
                        first_rect,
                        pointer_x,
                        pointer_y,
                        min_region_size,
                    ),
                    SplitterChild::Second => second.resize_splitter_if_changed_inner(
                        &path[1..],
                        second_rect,
                        pointer_x,
                        pointer_y,
                        min_region_size,
                    ),
                }
            }
        }
    }

    fn set_splitter_ratio_inner(
        &mut self,
        path: &[SplitterChild],
        replacement_ratio: SplitRatio,
    ) -> DomainResult<()> {
        match self {
            Self::Region { .. } => Err(DomainError::SplitterNotFound),
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if path.is_empty() {
                    *ratio = replacement_ratio;
                    return Ok(());
                }

                match path[0] {
                    SplitterChild::First => {
                        first.set_splitter_ratio_inner(&path[1..], replacement_ratio)
                    }
                    SplitterChild::Second => {
                        second.set_splitter_ratio_inner(&path[1..], replacement_ratio)
                    }
                }
            }
        }
    }

    fn resize_splitter_would_change_inner(
        &self,
        path: &[SplitterChild],
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<bool> {
        match self {
            Self::Region { .. } => Err(DomainError::SplitterNotFound),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if path.is_empty() {
                    let next_ratio = split_ratio_from_pointer(
                        *direction,
                        bounds,
                        pointer_x,
                        pointer_y,
                        min_region_size,
                    )?;
                    return Ok(next_ratio != *ratio);
                }

                let (first_rect, second_rect) =
                    bounds.split(*direction, *ratio, min_region_size)?;
                match path[0] {
                    SplitterChild::First => first.resize_splitter_would_change_inner(
                        &path[1..],
                        first_rect,
                        pointer_x,
                        pointer_y,
                        min_region_size,
                    ),
                    SplitterChild::Second => second.resize_splitter_would_change_inner(
                        &path[1..],
                        second_rect,
                        pointer_x,
                        pointer_y,
                        min_region_size,
                    ),
                }
            }
        }
    }
}

fn split_ratio_from_pointer(
    direction: SplitDirection,
    bounds: Rect,
    pointer_x: i32,
    pointer_y: i32,
    min_region_size: i32,
) -> DomainResult<SplitRatio> {
    let (available, first_child) = match direction {
        SplitDirection::Vertical => (
            bounds.width(),
            pointer_x
                .checked_sub(bounds.left())
                .ok_or(DomainError::CoordinateOverflow)?,
        ),
        SplitDirection::Horizontal => (
            bounds.height(),
            pointer_y
                .checked_sub(bounds.top())
                .ok_or(DomainError::CoordinateOverflow)?,
        ),
    };
    SplitRatio::from_first_child_size(direction, available, first_child, min_region_size)
}

pub trait RegionIdAllocator {
    fn allocate_region(&mut self) -> DomainResult<RegionId>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalProgramSpec {
    executable_path: String,
    #[cfg(unix)]
    executable_path_unix_bytes: Option<Vec<u8>>,
    arguments: Vec<String>,
    title: Option<String>,
}

impl ExternalProgramSpec {
    pub fn new(executable_path: impl Into<String>, title: Option<String>) -> DomainResult<Self> {
        Self::new_with_arguments(executable_path, Vec::<String>::new(), title)
    }

    pub fn new_with_arguments<I, S>(
        executable_path: impl Into<String>,
        arguments: I,
        title: Option<String>,
    ) -> DomainResult<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ExternalProgramSpecInput::new(
            executable_path.into(),
            arguments.into_iter().map(Into::into).collect(),
            title,
        )
        .into_spec()
    }

    #[cfg(unix)]
    pub fn new_with_unix_executable_path_bytes(
        executable_path: impl Into<Vec<u8>>,
        title: Option<String>,
    ) -> DomainResult<Self> {
        Self::new_with_unix_executable_path_bytes_and_arguments(
            executable_path,
            Vec::<String>::new(),
            title,
        )
    }

    #[cfg(unix)]
    pub fn new_with_unix_executable_path_bytes_and_arguments<I, S>(
        executable_path: impl Into<Vec<u8>>,
        arguments: I,
        title: Option<String>,
    ) -> DomainResult<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let executable_path = executable_path.into();
        let display_path = String::from_utf8_lossy(&executable_path).into_owned();
        let executable_path_unix_bytes = if display_path.as_bytes() == executable_path.as_slice() {
            None
        } else {
            Some(executable_path)
        };

        ExternalProgramSpecInput::new(
            display_path,
            arguments.into_iter().map(Into::into).collect(),
            title,
        )
        .with_unix_executable_path_bytes(executable_path_unix_bytes)
        .into_spec()
    }

    pub fn executable_path(&self) -> &str {
        &self.executable_path
    }

    pub fn executable_path_os(&self) -> &OsStr {
        #[cfg(unix)]
        if let Some(executable_path) = &self.executable_path_unix_bytes {
            return OsStr::from_bytes(executable_path);
        }

        OsStr::new(&self.executable_path)
    }

    #[cfg(unix)]
    pub fn executable_path_unix_bytes(&self) -> Option<&[u8]> {
        self.executable_path_unix_bytes.as_deref()
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
}

struct ExternalProgramSpecInput {
    executable_path: String,
    #[cfg(unix)]
    executable_path_unix_bytes: Option<Vec<u8>>,
    arguments: Vec<String>,
    title: Option<String>,
}

impl ExternalProgramSpecInput {
    fn new(executable_path: String, arguments: Vec<String>, title: Option<String>) -> Self {
        Self {
            executable_path,
            #[cfg(unix)]
            executable_path_unix_bytes: None,
            arguments,
            title,
        }
    }

    #[cfg(unix)]
    fn with_unix_executable_path_bytes(mut self, executable_path: Option<Vec<u8>>) -> Self {
        self.executable_path_unix_bytes = executable_path;
        self
    }

    fn into_spec(self) -> DomainResult<ExternalProgramSpec> {
        #[cfg(unix)]
        match &self.executable_path_unix_bytes {
            Some(executable_path) => validate_program_executable_path_bytes(executable_path)?,
            None => validate_program_executable_path(&self.executable_path)?,
        }
        #[cfg(not(unix))]
        validate_program_executable_path(&self.executable_path)?;
        validate_program_arguments(&self.arguments)?;

        Ok(ExternalProgramSpec {
            executable_path: self.executable_path,
            #[cfg(unix)]
            executable_path_unix_bytes: self.executable_path_unix_bytes,
            arguments: self.arguments,
            title: Self::normalize_title(self.title),
        })
    }

    fn normalize_title(title: Option<String>) -> Option<String> {
        title.and_then(|title| {
            let title = title.trim().to_owned();
            if title.is_empty() { None } else { Some(title) }
        })
    }
}

fn validate_program_executable_path(path: &str) -> DomainResult<()> {
    if path.trim().is_empty() || path.contains('\0') {
        Err(DomainError::EmptyProgramExecutablePath)
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn validate_program_executable_path_bytes(path: &[u8]) -> DomainResult<()> {
    if path.is_empty() || path.contains(&0) {
        return Err(DomainError::EmptyProgramExecutablePath);
    }

    match std::str::from_utf8(path) {
        Ok(path) => validate_program_executable_path(path),
        Err(_) => Ok(()),
    }
}

fn validate_program_arguments(arguments: &[String]) -> DomainResult<()> {
    if arguments.iter().any(|argument| argument.contains('\0')) {
        Err(DomainError::InvalidProgramArgument)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabPreset {
    name: String,
    root: TabPresetNode,
}

impl TabPreset {
    pub fn new(name: impl Into<String>, root: TabPresetNode) -> DomainResult<Self> {
        let name = name.into();
        let name = normalize_tab_preset_name(&name);
        validate_tab_preset_name(&name)?;

        Ok(Self { name, root })
    }

    pub fn from_layout_and_programs(
        name: impl Into<String>,
        layout: &LayoutNode,
        programs_by_region_id: &HashMap<RegionId, ExternalProgramSpec>,
    ) -> DomainResult<Self> {
        Self::new(
            name,
            TabPresetNode::from_layout_node(layout, programs_by_region_id),
        )
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn rename(&mut self, name: impl Into<String>) -> DomainResult<()> {
        let name = name.into();
        let name = normalize_tab_preset_name(&name);
        validate_tab_preset_name(&name)?;
        self.name = name;
        Ok(())
    }

    pub const fn root(&self) -> &TabPresetNode {
        &self.root
    }

    pub fn to_layout_node_with_programs(
        &self,
        allocator: &mut impl RegionIdAllocator,
    ) -> DomainResult<(LayoutNode, Vec<TabPresetProgramPlacement>)> {
        let mut programs = Vec::new();
        let layout = self
            .root
            .to_layout_node_with_programs(allocator, &mut programs)?;
        Ok((layout, programs))
    }

    pub fn program_specs(&self) -> Vec<ExternalProgramSpec> {
        let mut programs = Vec::new();
        self.root.collect_program_specs(&mut programs);
        programs
    }

    pub fn replace_program_specs(
        &mut self,
        programs: impl IntoIterator<Item = ExternalProgramSpec>,
    ) -> usize {
        let mut programs = programs.into_iter();
        self.root.replace_program_specs(&mut programs)
    }
}

pub fn normalize_tab_preset_name(name: &str) -> String {
    name.trim().to_owned()
}

pub fn validate_tab_preset_name(name: &str) -> DomainResult<()> {
    if name.trim().is_empty() || name.contains('\0') {
        Err(DomainError::EmptyTabPresetName)
    } else {
        Ok(())
    }
}

pub fn upsert_tab_preset(tab_presets: &mut Vec<TabPreset>, preset: TabPreset) {
    if let Some(existing) = tab_presets
        .iter_mut()
        .find(|existing| existing.name() == preset.name())
    {
        *existing = preset;
    } else {
        tab_presets.push(preset);
    }
}

pub fn remove_tab_preset(tab_presets: &mut Vec<TabPreset>, name: &str) -> DomainResult<TabPreset> {
    let name = normalize_tab_preset_name(name);
    validate_tab_preset_name(&name)?;
    let index = tab_presets
        .iter()
        .position(|preset| preset.name() == name)
        .ok_or_else(|| DomainError::TabPresetNotFound(name.clone()))?;

    Ok(tab_presets.remove(index))
}

pub(crate) fn canonicalize_tab_presets(tab_presets: Vec<TabPreset>) -> Vec<TabPreset> {
    let mut canonical = Vec::with_capacity(tab_presets.len());
    let mut indices_by_name = HashMap::with_capacity(tab_presets.len());
    for preset in tab_presets {
        if let Some(index) = indices_by_name.get(preset.name()).copied() {
            canonical[index] = preset;
        } else {
            indices_by_name.insert(preset.name().to_owned(), canonical.len());
            canonical.push(preset);
        }
    }
    canonical
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabPresetNode {
    Region {
        program: Option<ExternalProgramSpec>,
    },
    Split {
        direction: SplitDirection,
        ratio: SplitRatio,
        first: Box<TabPresetNode>,
        second: Box<TabPresetNode>,
    },
}

impl TabPresetNode {
    pub fn from_layout_node(
        layout: &LayoutNode,
        programs_by_region_id: &HashMap<RegionId, ExternalProgramSpec>,
    ) -> Self {
        match layout {
            LayoutNode::Region { id } => Self::Region {
                program: programs_by_region_id.get(id).cloned(),
            },
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => Self::Split {
                direction: *direction,
                ratio: *ratio,
                first: Box::new(Self::from_layout_node(first, programs_by_region_id)),
                second: Box::new(Self::from_layout_node(second, programs_by_region_id)),
            },
        }
    }

    fn to_layout_node_with_programs(
        &self,
        allocator: &mut impl RegionIdAllocator,
        programs: &mut Vec<TabPresetProgramPlacement>,
    ) -> DomainResult<LayoutNode> {
        match self {
            Self::Region { program } => {
                let region_id = allocator.allocate_region()?;
                if let Some(program) = program {
                    programs.push(TabPresetProgramPlacement::new(region_id, program.clone()));
                }
                Ok(LayoutNode::single_region(region_id))
            }
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => Ok(LayoutNode::Split {
                direction: *direction,
                ratio: *ratio,
                first: Box::new(first.to_layout_node_with_programs(allocator, programs)?),
                second: Box::new(second.to_layout_node_with_programs(allocator, programs)?),
            }),
        }
    }

    fn collect_program_specs(&self, programs: &mut Vec<ExternalProgramSpec>) {
        match self {
            Self::Region { program } => {
                if let Some(program) = program {
                    programs.push(program.clone());
                }
            }
            Self::Split { first, second, .. } => {
                first.collect_program_specs(programs);
                second.collect_program_specs(programs);
            }
        }
    }

    fn replace_program_specs(
        &mut self,
        programs: &mut impl Iterator<Item = ExternalProgramSpec>,
    ) -> usize {
        match self {
            Self::Region { program } => {
                if let Some(program) = program {
                    if let Some(edited) = programs.next() {
                        *program = edited;
                        1
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
            Self::Split { first, second, .. } => {
                first.replace_program_specs(programs) + second.replace_program_specs(programs)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabPresetProgramPlacement {
    region_id: RegionId,
    program: ExternalProgramSpec,
}

impl TabPresetProgramPlacement {
    pub fn new(region_id: RegionId, program: ExternalProgramSpec) -> Self {
        Self { region_id, program }
    }

    pub const fn region_id(&self) -> RegionId {
        self.region_id
    }

    pub const fn program(&self) -> &ExternalProgramSpec {
        &self.program
    }
}

fn splitter_hit_rect(
    direction: SplitDirection,
    first_rect: Rect,
    split_bounds: Rect,
    tolerance: i32,
) -> DomainResult<Rect> {
    let thickness = tolerance
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or(DomainError::CoordinateOverflow)?;

    match direction {
        SplitDirection::Vertical => {
            let splitter_x = first_rect
                .left()
                .checked_add(first_rect.width())
                .ok_or(DomainError::CoordinateOverflow)?;
            Rect::new(
                splitter_x
                    .checked_sub(tolerance)
                    .ok_or(DomainError::CoordinateOverflow)?,
                split_bounds.top(),
                thickness,
                split_bounds.height(),
            )
        }
        SplitDirection::Horizontal => {
            let splitter_y = first_rect
                .top()
                .checked_add(first_rect.height())
                .ok_or(DomainError::CoordinateOverflow)?;
            Rect::new(
                split_bounds.left(),
                splitter_y
                    .checked_sub(tolerance)
                    .ok_or(DomainError::CoordinateOverflow)?,
                split_bounds.width(),
                thickness,
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowDisplayState {
    Hidden,
    Normal,
    Minimized,
    Maximized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZOrderHint {
    value: isize,
}

impl ZOrderHint {
    pub const fn new(value: isize) -> Self {
        Self { value }
    }

    pub const fn value(self) -> isize {
        self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSnapshot {
    hwnd: WindowHandle,
    rect: Rect,
    display_state: WindowDisplayState,
    identity: Option<WindowIdentity>,
    owner: Option<WindowHandle>,
    z_order_hint: Option<ZOrderHint>,
    style: Option<u32>,
    ex_style: Option<u32>,
}

impl WindowSnapshot {
    pub const fn new(hwnd: WindowHandle, rect: Rect, display_state: WindowDisplayState) -> Self {
        Self {
            hwnd,
            rect,
            display_state,
            identity: None,
            owner: None,
            z_order_hint: None,
            style: None,
            ex_style: None,
        }
    }

    pub const fn hwnd(&self) -> WindowHandle {
        self.hwnd
    }

    pub const fn rect(&self) -> Rect {
        self.rect
    }

    pub const fn display_state(&self) -> WindowDisplayState {
        self.display_state
    }

    pub const fn identity(&self) -> Option<WindowIdentity> {
        self.identity
    }

    pub const fn owner(&self) -> Option<WindowHandle> {
        self.owner
    }

    pub const fn z_order_hint(&self) -> Option<ZOrderHint> {
        self.z_order_hint
    }

    pub const fn style(&self) -> Option<u32> {
        self.style
    }

    pub const fn ex_style(&self) -> Option<u32> {
        self.ex_style
    }

    pub const fn with_rect(mut self, rect: Rect) -> Self {
        self.rect = rect;
        self
    }

    pub const fn with_display_state(mut self, display_state: WindowDisplayState) -> Self {
        self.display_state = display_state;
        self
    }

    pub const fn with_z_order_hint(mut self, z_order_hint: ZOrderHint) -> Self {
        self.z_order_hint = Some(z_order_hint);
        self
    }

    pub const fn with_identity(mut self, identity: WindowIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    pub const fn with_owner(mut self, owner: WindowHandle) -> Self {
        self.owner = Some(owner);
        self
    }

    pub const fn with_style(mut self, style: u32) -> Self {
        self.style = Some(style);
        self
    }

    pub const fn with_ex_style(mut self, ex_style: u32) -> Self {
        self.ex_style = Some(ex_style);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    tab_id: TabId,
    region_id: RegionId,
    hwnd: WindowHandle,
    snapshot: WindowSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemovedPlacement {
    index: usize,
    placement: Placement,
}

impl RemovedPlacement {
    const fn new(index: usize, placement: Placement) -> Self {
        Self { index, placement }
    }

    pub(crate) const fn tab_id(&self) -> TabId {
        self.placement.tab_id()
    }

    pub(crate) const fn region_id(&self) -> RegionId {
        self.placement.region_id()
    }

    pub(crate) const fn hwnd(&self) -> WindowHandle {
        self.placement.hwnd()
    }
}

impl Placement {
    pub fn new(
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
        snapshot: WindowSnapshot,
    ) -> DomainResult<Self> {
        validate_placement_snapshot(hwnd, &snapshot)?;

        Ok(Self {
            tab_id,
            region_id,
            hwnd,
            snapshot,
        })
    }

    pub const fn tab_id(&self) -> TabId {
        self.tab_id
    }

    pub const fn region_id(&self) -> RegionId {
        self.region_id
    }

    pub const fn hwnd(&self) -> WindowHandle {
        self.hwnd
    }

    pub const fn snapshot(&self) -> &WindowSnapshot {
        &self.snapshot
    }
}

fn validate_placement_snapshot(hwnd: WindowHandle, snapshot: &WindowSnapshot) -> DomainResult<()> {
    if snapshot.hwnd() != hwnd {
        Err(DomainError::WindowSnapshotMismatch {
            placement: hwnd,
            snapshot: snapshot.hwnd(),
        })
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedWindowRestorePolicy {
    SessionOnlyNoAutoRestore,
}

impl SavedWindowRestorePolicy {
    pub const fn allows_auto_restore(self) -> bool {
        match self {
            Self::SessionOnlyNoAutoRestore => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedPlacement {
    region_id: RegionId,
    hwnd: WindowHandle,
    snapshot: WindowSnapshot,
    restore_policy: SavedWindowRestorePolicy,
}

impl SavedPlacement {
    pub fn new(
        region_id: RegionId,
        hwnd: WindowHandle,
        snapshot: WindowSnapshot,
        restore_policy: SavedWindowRestorePolicy,
    ) -> DomainResult<Self> {
        validate_placement_snapshot(hwnd, &snapshot)?;

        Ok(Self {
            region_id,
            hwnd,
            snapshot,
            restore_policy,
        })
    }

    pub fn from_live_placement(placement: &Placement) -> Self {
        Self {
            region_id: placement.region_id(),
            hwnd: placement.hwnd(),
            snapshot: placement.snapshot().clone(),
            restore_policy: SavedWindowRestorePolicy::SessionOnlyNoAutoRestore,
        }
    }

    pub const fn region_id(&self) -> RegionId {
        self.region_id
    }

    pub const fn hwnd(&self) -> WindowHandle {
        self.hwnd
    }

    pub const fn snapshot(&self) -> &WindowSnapshot {
        &self.snapshot
    }

    pub const fn restore_policy(&self) -> SavedWindowRestorePolicy {
        self.restore_policy
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabSettings {
    id: TabId,
    name: String,
    layout: LayoutNode,
    placements: Vec<SavedPlacement>,
    max_region_id: Option<u64>,
}

impl TabSettings {
    pub fn new(
        id: TabId,
        name: impl Into<String>,
        layout: LayoutNode,
        placements: Vec<SavedPlacement>,
    ) -> DomainResult<Self> {
        let name = name.into();
        validate_tab_name(&name)?;

        let validation = TabSettingsValidation::validate(&layout, &placements)?;

        Ok(Self {
            id,
            name,
            layout,
            placements,
            max_region_id: validation.max_region_id(),
        })
    }

    fn from_tab(tab: &Tab) -> DomainResult<Self> {
        let placements = tab
            .placements()
            .iter()
            .map(SavedPlacement::from_live_placement)
            .collect();
        Self::new(
            tab.id(),
            tab.name().to_owned(),
            tab.layout().clone(),
            placements,
        )
    }

    pub const fn id(&self) -> TabId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn layout(&self) -> &LayoutNode {
        &self.layout
    }

    pub fn placements(&self) -> &[SavedPlacement] {
        &self.placements
    }

    const fn max_region_id(&self) -> Option<u64> {
        self.max_region_id
    }

    fn into_runtime_tab_without_placements(self) -> Tab {
        Tab {
            id: self.id,
            name: self.name,
            layout: self.layout,
            placements: Vec::new(),
        }
    }
}

struct TabSettingsValidation {
    max_region_id: Option<u64>,
}

impl TabSettingsValidation {
    fn validate(layout: &LayoutNode, placements: &[SavedPlacement]) -> DomainResult<Self> {
        let layout_regions = LayoutRegionSummary::collect(layout)?;
        validate_saved_placements(&layout_regions, placements)?;
        Ok(Self {
            max_region_id: layout_regions.max_region_id(),
        })
    }

    const fn max_region_id(&self) -> Option<u64> {
        self.max_region_id
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSettings {
    tabs: Vec<TabSettings>,
    active_tab_id: Option<TabId>,
    next_tab_id: u64,
    next_region_id: u64,
    tab_presets: Vec<TabPreset>,
    options: WorkspaceOptions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UiLanguage {
    #[default]
    English,
    Korean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceOptions {
    dock_hidden_workspace_ui: bool,
    ui_language: UiLanguage,
}

impl WorkspaceOptions {
    pub const fn new(dock_hidden_workspace_ui: bool) -> Self {
        Self::new_with_language(dock_hidden_workspace_ui, UiLanguage::English)
    }

    pub const fn new_with_language(
        dock_hidden_workspace_ui: bool,
        ui_language: UiLanguage,
    ) -> Self {
        Self {
            dock_hidden_workspace_ui,
            ui_language,
        }
    }

    pub const fn dock_hidden_workspace_ui(self) -> bool {
        self.dock_hidden_workspace_ui
    }

    pub const fn ui_language(self) -> UiLanguage {
        self.ui_language
    }

    pub const fn with_dock_hidden_workspace_ui(self, enabled: bool) -> Self {
        Self {
            dock_hidden_workspace_ui: enabled,
            ui_language: self.ui_language,
        }
    }

    pub const fn with_ui_language(self, ui_language: UiLanguage) -> Self {
        Self {
            dock_hidden_workspace_ui: self.dock_hidden_workspace_ui,
            ui_language,
        }
    }
}

impl Default for WorkspaceOptions {
    fn default() -> Self {
        Self::new(false)
    }
}

impl WorkspaceSettings {
    pub fn new(
        tabs: Vec<TabSettings>,
        active_tab_id: Option<TabId>,
        next_tab_id: u64,
        next_region_id: u64,
    ) -> DomainResult<Self> {
        Self::new_with_tab_presets_and_options(
            tabs,
            active_tab_id,
            next_tab_id,
            next_region_id,
            Vec::new(),
            WorkspaceOptions::default(),
        )
    }

    pub fn new_with_tab_presets_and_options(
        tabs: Vec<TabSettings>,
        active_tab_id: Option<TabId>,
        next_tab_id: u64,
        next_region_id: u64,
        tab_presets: Vec<TabPreset>,
        options: WorkspaceOptions,
    ) -> DomainResult<Self> {
        validate_unique_tab_ids(&tabs)?;
        let tab_presets = canonicalize_tab_presets(tab_presets);

        let active_tab_id = match active_tab_id {
            Some(active_tab_id) => {
                if !tabs.iter().any(|tab| tab.id() == active_tab_id) {
                    return Err(DomainError::TabNotFound(active_tab_id));
                }
                Some(active_tab_id)
            }
            None => tabs.first().map(TabSettings::id),
        };

        Ok(Self {
            tabs,
            active_tab_id,
            next_tab_id,
            next_region_id,
            tab_presets,
            options,
        })
    }

    pub fn tabs(&self) -> &[TabSettings] {
        &self.tabs
    }

    pub const fn active_tab_id(&self) -> Option<TabId> {
        self.active_tab_id
    }

    pub const fn next_tab_id(&self) -> u64 {
        self.next_tab_id
    }

    pub const fn next_region_id(&self) -> u64 {
        self.next_region_id
    }

    pub fn tab_presets(&self) -> &[TabPreset] {
        &self.tab_presets
    }

    pub const fn options(&self) -> WorkspaceOptions {
        self.options
    }

    pub fn set_options(&mut self, options: WorkspaceOptions) {
        self.options = options;
    }

    pub fn saved_placement_count(&self) -> usize {
        self.tabs.iter().map(|tab| tab.placements().len()).sum()
    }
}

fn validate_saved_placements(
    layout_regions: &LayoutRegionSummary,
    placements: &[SavedPlacement],
) -> DomainResult<()> {
    let mut occupied = HashSet::new();

    for placement in placements {
        let region_id = placement.region_id();
        if !layout_regions.contains(region_id) {
            return Err(DomainError::RegionNotFound(region_id));
        }

        if !occupied.insert(region_id) {
            return Err(DomainError::RegionAlreadyOccupied(region_id));
        }
    }

    Ok(())
}

struct LayoutRegionSummary {
    ids: HashSet<RegionId>,
    max_region_id: Option<u64>,
}

impl LayoutRegionSummary {
    fn collect(layout: &LayoutNode) -> DomainResult<Self> {
        let mut ids = HashSet::new();
        let mut max_region_id: Option<u64> = None;
        layout.visit_unique_region_ids(&mut ids, &mut |id| {
            let value = id.value();
            max_region_id = Some(max_region_id.map_or(value, |max| max.max(value)));
        })?;

        Ok(Self { ids, max_region_id })
    }

    fn contains(&self, region_id: RegionId) -> bool {
        self.ids.contains(&region_id)
    }

    const fn max_region_id(&self) -> Option<u64> {
        self.max_region_id
    }
}

fn validate_unique_tab_ids(tabs: &[TabSettings]) -> DomainResult<()> {
    let mut tab_ids = HashSet::new();

    for tab in tabs {
        if !tab_ids.insert(tab.id()) {
            return Err(DomainError::DuplicateTab(tab.id()));
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    id: TabId,
    name: String,
    layout: LayoutNode,
    placements: Vec<Placement>,
}

impl Tab {
    pub fn new(id: TabId, name: impl Into<String>, root_region: RegionId) -> DomainResult<Self> {
        let name = name.into();
        validate_tab_name(&name)?;

        Ok(Self {
            id,
            name,
            layout: LayoutNode::single_region(root_region),
            placements: Vec::new(),
        })
    }

    pub const fn id(&self) -> TabId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn layout(&self) -> &LayoutNode {
        &self.layout
    }

    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    pub fn rename(&mut self, name: impl Into<String>) -> DomainResult<()> {
        let name = name.into();
        validate_tab_name(&name)?;

        self.name = name;
        Ok(())
    }

    pub fn split_region(
        &mut self,
        region_id: RegionId,
        direction: SplitDirection,
        new_region: RegionId,
        ratio: SplitRatio,
    ) -> DomainResult<()> {
        self.layout
            .split_region(region_id, direction, new_region, ratio)
    }

    pub fn delete_region(&mut self, region_id: RegionId) -> DomainResult<Option<Placement>> {
        self.layout.delete_region(region_id)?;
        Ok(self.remove_placement_by_region(region_id))
    }

    pub fn layout_rects(
        &self,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        self.layout.region_rects(bounds, min_region_size)
    }

    pub fn find_region_rect(
        &self,
        region_id: RegionId,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Rect> {
        self.layout
            .find_region_rect(region_id, bounds, min_region_size)
    }

    pub fn region_and_splitter_rects(
        &self,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<(Vec<RegionRect>, Vec<SplitterRect>)> {
        self.layout
            .region_and_splitter_rects(bounds, tolerance, min_region_size)
    }

    pub fn region_and_splitter_rects_into(
        &self,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
        regions: &mut Vec<RegionRect>,
        splitters: &mut Vec<SplitterRect>,
    ) -> DomainResult<()> {
        self.layout.region_and_splitter_rects_into(
            bounds,
            tolerance,
            min_region_size,
            regions,
            splitters,
        )
    }

    pub fn hit_test_region(
        &self,
        bounds: Rect,
        x: i32,
        y: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<RegionId>> {
        self.layout.hit_test(bounds, x, y, min_region_size)
    }

    pub fn splitter_rects(
        &self,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<Vec<SplitterRect>> {
        self.layout
            .splitter_rects(bounds, tolerance, min_region_size)
    }

    pub fn splitter_rects_into(
        &self,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
        splitters: &mut Vec<SplitterRect>,
    ) -> DomainResult<()> {
        self.layout
            .splitter_rects_into(bounds, tolerance, min_region_size, splitters)
    }

    pub fn hit_test_splitter(
        &self,
        bounds: Rect,
        x: i32,
        y: i32,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<SplitterRect>> {
        self.layout
            .hit_test_splitter(bounds, x, y, tolerance, min_region_size)
    }

    pub fn resize_splitter(
        &mut self,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<()> {
        self.layout
            .resize_splitter(path, bounds, pointer_x, pointer_y, min_region_size)
    }

    pub(crate) fn resize_splitter_if_changed(
        &mut self,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<SplitterResizeRollback>> {
        self.layout
            .resize_splitter_if_changed(path, bounds, pointer_x, pointer_y, min_region_size)
    }

    pub(crate) fn rollback_splitter_resize(
        &mut self,
        path: &SplitterPath,
        rollback: SplitterResizeRollback,
    ) -> DomainResult<()> {
        self.layout.rollback_splitter_resize(path, rollback)
    }

    pub(crate) fn region_rects_before_splitter_resize(
        &self,
        path: &SplitterPath,
        bounds: Rect,
        rollback: SplitterResizeRollback,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        self.layout
            .region_rects_before_splitter_resize(path, bounds, rollback, min_region_size)
    }

    #[cfg(any(test, target_os = "windows"))]
    pub(crate) fn region_rects_for_splitter_resize(
        &self,
        path: &SplitterPath,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        self.layout
            .region_rects_for_splitter_resize(path, bounds, min_region_size)
    }

    pub fn resize_splitter_would_change(
        &self,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<bool> {
        self.layout.resize_splitter_would_change(
            path,
            bounds,
            pointer_x,
            pointer_y,
            min_region_size,
        )
    }

    pub fn add_placement(&mut self, placement: Placement) -> DomainResult<()> {
        if placement.tab_id() != self.id {
            return Err(DomainError::PlacementTabMismatch {
                expected: self.id,
                actual: placement.tab_id(),
            });
        }

        self.ensure_region_available(placement.region_id(), placement.hwnd())?;
        self.placements.push(placement);
        Ok(())
    }

    pub fn remove_placement_by_region(&mut self, region_id: RegionId) -> Option<Placement> {
        let index = self
            .placements
            .iter()
            .position(|placement| placement.region_id() == region_id)?;
        Some(self.placements.remove(index))
    }

    pub(crate) fn remove_placement_by_region_for_rollback(
        &mut self,
        region_id: RegionId,
    ) -> Option<RemovedPlacement> {
        let index = self
            .placements
            .iter()
            .position(|placement| placement.region_id() == region_id)?;
        Some(RemovedPlacement::new(index, self.placements.remove(index)))
    }

    pub(crate) fn restore_removed_placement(
        &mut self,
        removed: RemovedPlacement,
    ) -> DomainResult<()> {
        if removed.tab_id() != self.id {
            return Err(DomainError::PlacementTabMismatch {
                expected: self.id,
                actual: removed.tab_id(),
            });
        }

        self.ensure_region_available(removed.region_id(), removed.hwnd())?;
        let index = removed.index.min(self.placements.len());
        self.placements.insert(index, removed.placement);
        Ok(())
    }

    pub fn move_placement(
        &mut self,
        source_region_id: RegionId,
        target_region_id: RegionId,
    ) -> DomainResult<bool> {
        if !self.ensure_can_move_placement(source_region_id, target_region_id)? {
            return Ok(false);
        }

        let Some(placement) = self
            .placements
            .iter_mut()
            .find(|placement| placement.region_id() == source_region_id)
        else {
            return Err(DomainError::PlacementNotFound {
                tab_id: self.id,
                region_id: source_region_id,
            });
        };

        placement.region_id = target_region_id;
        Ok(true)
    }

    pub fn take_placements(&mut self) -> Vec<Placement> {
        std::mem::take(&mut self.placements)
    }

    fn ensure_can_move_placement(
        &self,
        source_region_id: RegionId,
        target_region_id: RegionId,
    ) -> DomainResult<bool> {
        if !self.layout.contains_region(target_region_id) {
            return Err(DomainError::RegionNotFound(target_region_id));
        }

        if !self
            .placements
            .iter()
            .any(|placement| placement.region_id() == source_region_id)
        {
            return Err(DomainError::PlacementNotFound {
                tab_id: self.id,
                region_id: source_region_id,
            });
        }

        if source_region_id == target_region_id {
            return Ok(false);
        }

        if self
            .placements
            .iter()
            .any(|placement| placement.region_id() == target_region_id)
        {
            return Err(DomainError::RegionAlreadyOccupied(target_region_id));
        }

        Ok(true)
    }

    fn ensure_region_available(&self, region_id: RegionId, hwnd: WindowHandle) -> DomainResult<()> {
        if !self.layout.contains_region(region_id) {
            return Err(DomainError::RegionNotFound(region_id));
        }

        if self
            .placements
            .iter()
            .any(|placement| placement.region_id() == region_id)
        {
            return Err(DomainError::RegionAlreadyOccupied(region_id));
        }

        if self
            .placements
            .iter()
            .any(|placement| placement.hwnd() == hwnd)
        {
            return Err(DomainError::WindowAlreadyPlaced(hwnd));
        }

        Ok(())
    }
}

fn validate_tab_name(name: &str) -> DomainResult<()> {
    if name.trim().is_empty() || name.contains('\0') {
        Err(DomainError::EmptyTabName)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveTabChange {
    previous: Option<TabId>,
    current: TabId,
}

impl ActiveTabChange {
    pub const fn new(previous: Option<TabId>, current: TabId) -> Self {
        Self { previous, current }
    }

    pub const fn previous(self) -> Option<TabId> {
        self.previous
    }

    pub const fn current(self) -> TabId {
        self.current
    }
}

#[derive(Debug, Clone)]
pub struct TabDeletion {
    removed_tab: Tab,
    removed_index: usize,
    previous_active_tab: Option<TabId>,
    current_active_tab: Option<TabId>,
    previous_next_tab_id: u64,
    previous_next_region_id: u64,
}

impl PartialEq for TabDeletion {
    fn eq(&self, other: &Self) -> bool {
        self.removed_tab == other.removed_tab
            && self.previous_active_tab == other.previous_active_tab
            && self.current_active_tab == other.current_active_tab
    }
}

impl TabDeletion {
    pub const fn new(
        removed_tab: Tab,
        previous_active_tab: Option<TabId>,
        current_active_tab: Option<TabId>,
    ) -> Self {
        Self {
            removed_tab,
            removed_index: 0,
            previous_active_tab,
            current_active_tab,
            previous_next_tab_id: EMPTY_WORKSPACE_NEXT_TAB_ID,
            previous_next_region_id: EMPTY_WORKSPACE_NEXT_REGION_ID,
        }
    }

    const fn with_removed_index(
        removed_index: usize,
        removed_tab: Tab,
        previous_active_tab: Option<TabId>,
        current_active_tab: Option<TabId>,
        previous_next_tab_id: u64,
        previous_next_region_id: u64,
    ) -> Self {
        Self {
            removed_tab,
            removed_index,
            previous_active_tab,
            current_active_tab,
            previous_next_tab_id,
            previous_next_region_id,
        }
    }

    pub const fn removed_tab(&self) -> &Tab {
        &self.removed_tab
    }

    pub const fn previous_active_tab(&self) -> Option<TabId> {
        self.previous_active_tab
    }

    pub const fn current_active_tab(&self) -> Option<TabId> {
        self.current_active_tab
    }

    pub fn into_removed_tab(self) -> Tab {
        self.removed_tab
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    tabs: Vec<Tab>,
    active_tab_id: Option<TabId>,
    next_tab_id: u64,
    next_region_id: u64,
}

// Tab lifecycle and selection.
impl Workspace {
    pub const fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab_id: None,
            next_tab_id: EMPTY_WORKSPACE_NEXT_TAB_ID,
            next_region_id: EMPTY_WORKSPACE_NEXT_REGION_ID,
        }
    }

    pub fn add_tab(&mut self, name: impl Into<String>) -> DomainResult<TabId> {
        let name = name.into();
        validate_tab_name(&name)?;

        let tab_id = TabId::new(self.next_tab_id);
        let root_region = RegionId::new(self.next_region_id);
        let next_tab_id = self
            .next_tab_id
            .checked_add(1)
            .ok_or(DomainError::IdExhausted("tab"))?;
        let next_region_id = self
            .next_region_id
            .checked_add(1)
            .ok_or(DomainError::IdExhausted("region"))?;
        let tab = Tab::new(tab_id, name, root_region)?;

        self.next_tab_id = next_tab_id;
        self.next_region_id = next_region_id;
        self.tabs.push(tab);

        if self.active_tab_id.is_none() {
            self.active_tab_id = Some(tab_id);
        }

        Ok(tab_id)
    }

    pub fn delete_tab(&mut self, tab_id: TabId) -> DomainResult<TabDeletion> {
        let index = self.tab_index(tab_id)?;
        let previous_active_tab = self.active_tab_id;
        let previous_next_tab_id = self.next_tab_id;
        let previous_next_region_id = self.next_region_id;
        let removed_tab = self.tabs.remove(index);

        if previous_active_tab == Some(tab_id) {
            self.active_tab_id = self.active_tab_after_removed_index(index);
        }

        if self.tabs.is_empty() {
            self.reset_empty_id_counters();
        }

        Ok(TabDeletion::with_removed_index(
            index,
            removed_tab,
            previous_active_tab,
            self.active_tab_id,
            previous_next_tab_id,
            previous_next_region_id,
        ))
    }

    pub(crate) fn restore_deleted_tab(&mut self, deletion: TabDeletion) {
        let TabDeletion {
            removed_tab,
            removed_index,
            previous_active_tab,
            current_active_tab: _,
            previous_next_tab_id,
            previous_next_region_id,
        } = deletion;
        let insert_index = removed_index.min(self.tabs.len());
        self.tabs.insert(insert_index, removed_tab);
        self.active_tab_id = previous_active_tab;
        self.next_tab_id = previous_next_tab_id;
        self.next_region_id = previous_next_region_id;
    }

    pub fn rename_tab(&mut self, tab_id: TabId, name: impl Into<String>) -> DomainResult<()> {
        self.tab_mut(tab_id)?.rename(name)
    }

    pub fn reorder_tab_before(
        &mut self,
        tab_id: TabId,
        before_tab_id: Option<TabId>,
    ) -> DomainResult<bool> {
        let from_index = self.tab_index(tab_id)?;

        if before_tab_id == Some(tab_id) {
            return Ok(false);
        }

        let destination_index = match before_tab_id {
            Some(before_tab_id) => self.tab_index(before_tab_id)?,
            None => self.tabs.len(),
        };

        if destination_index == from_index || destination_index == from_index + 1 {
            return Ok(false);
        }

        let tab = self.tabs.remove(from_index);
        let insert_index = if destination_index > from_index {
            destination_index - 1
        } else {
            destination_index
        };
        self.tabs.insert(insert_index, tab);

        Ok(true)
    }

    pub fn set_active_tab(&mut self, tab_id: TabId) -> DomainResult<ActiveTabChange> {
        if !self.has_tab(tab_id) {
            return Err(DomainError::TabNotFound(tab_id));
        }

        let previous = self.active_tab_id;
        self.active_tab_id = Some(tab_id);
        Ok(ActiveTabChange::new(previous, tab_id))
    }

    pub const fn active_tab_id(&self) -> Option<TabId> {
        self.active_tab_id
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub const fn next_tab_id(&self) -> u64 {
        self.next_tab_id
    }

    pub const fn next_region_id(&self) -> u64 {
        self.next_region_id
    }
}

// Settings and tab preset conversion.
impl Workspace {
    pub fn to_settings(&self) -> DomainResult<WorkspaceSettings> {
        self.to_settings_with_tab_presets(Vec::new())
    }

    pub fn to_settings_with_tab_presets(
        &self,
        tab_presets: Vec<TabPreset>,
    ) -> DomainResult<WorkspaceSettings> {
        let tabs = self
            .tabs
            .iter()
            .map(TabSettings::from_tab)
            .collect::<DomainResult<Vec<_>>>()?;

        WorkspaceSettings::new_with_tab_presets_and_options(
            tabs,
            self.active_tab_id,
            self.next_tab_id,
            self.next_region_id,
            tab_presets,
            WorkspaceOptions::default(),
        )
    }

    pub fn layout_and_programs_from_tab_preset(
        &mut self,
        preset: &TabPreset,
    ) -> DomainResult<(LayoutNode, Vec<TabPresetProgramPlacement>)> {
        self.with_next_region_id_rollback(|workspace| {
            preset.to_layout_node_with_programs(workspace)
        })
    }

    pub fn from_settings_layout_only(settings: WorkspaceSettings) -> DomainResult<(Self, usize)> {
        let (workspace, saved_placement_count, _) =
            Self::from_settings_layout_only_preserving_presets(settings)?;
        Ok((workspace, saved_placement_count))
    }

    pub(crate) fn from_settings_layout_only_preserving_presets(
        settings: WorkspaceSettings,
    ) -> DomainResult<(Self, usize, Vec<TabPreset>)> {
        let saved_placement_count = settings.saved_placement_count();
        let WorkspaceSettings {
            tabs,
            active_tab_id,
            next_tab_id,
            next_region_id,
            tab_presets,
            options: _,
        } = settings;

        validate_unique_tab_ids(&tabs)?;

        let (runtime_tabs, max_tab_id, max_region_id) = Self::runtime_tabs_from_settings(tabs);
        Self::validate_active_tab(active_tab_id, &runtime_tabs)?;

        let next_tab_id =
            next_id_after_max(next_tab_id, max_tab_id, EMPTY_WORKSPACE_NEXT_TAB_ID, "tab")?;
        let next_region_id = next_id_after_max(
            next_region_id,
            max_region_id,
            EMPTY_WORKSPACE_NEXT_REGION_ID,
            "region",
        )?;

        Ok((
            Self {
                tabs: runtime_tabs,
                active_tab_id,
                next_tab_id,
                next_region_id,
            },
            saved_placement_count,
            tab_presets,
        ))
    }
}

// Runtime tab access and layout mutation.
impl Workspace {
    pub fn tab(&self, tab_id: TabId) -> DomainResult<&Tab> {
        let index = self.tab_index(tab_id)?;
        Ok(&self.tabs[index])
    }

    pub fn tab_mut(&mut self, tab_id: TabId) -> DomainResult<&mut Tab> {
        let index = self.tab_index(tab_id)?;
        Ok(&mut self.tabs[index])
    }

    pub fn split_region(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        direction: SplitDirection,
    ) -> DomainResult<RegionId> {
        let index = self.tab_index(tab_id)?;
        if !self.tabs[index].layout().contains_region(region_id) {
            return Err(DomainError::RegionNotFound(region_id));
        }

        self.with_next_region_id_rollback(|workspace| {
            let new_region = workspace.allocate_region_id()?;
            workspace.tabs[index].split_region(
                region_id,
                direction,
                new_region,
                DEFAULT_SPLIT_RATIO,
            )?;
            Ok(new_region)
        })
    }

    pub fn delete_region(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
    ) -> DomainResult<Option<Placement>> {
        self.tab_mut(tab_id)?.delete_region(region_id)
    }

    pub fn resize_splitter(
        &mut self,
        tab_id: TabId,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<()> {
        self.tab_mut(tab_id)?
            .resize_splitter(path, bounds, pointer_x, pointer_y, min_region_size)
    }

    pub(crate) fn resize_splitter_if_changed(
        &mut self,
        tab_id: TabId,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<SplitterResizeRollback>> {
        self.tab_mut(tab_id)?.resize_splitter_if_changed(
            path,
            bounds,
            pointer_x,
            pointer_y,
            min_region_size,
        )
    }

    pub(crate) fn rollback_splitter_resize(
        &mut self,
        tab_id: TabId,
        path: &SplitterPath,
        rollback: SplitterResizeRollback,
    ) -> DomainResult<()> {
        self.tab_mut(tab_id)?
            .rollback_splitter_resize(path, rollback)
    }

    pub(crate) fn region_rects_before_splitter_resize(
        &self,
        tab_id: TabId,
        path: &SplitterPath,
        bounds: Rect,
        rollback: SplitterResizeRollback,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        self.tab(tab_id)?.region_rects_before_splitter_resize(
            path,
            bounds,
            rollback,
            min_region_size,
        )
    }

    #[cfg(any(test, target_os = "windows"))]
    pub(crate) fn region_rects_for_splitter_resize(
        &self,
        tab_id: TabId,
        path: &SplitterPath,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        self.tab(tab_id)?
            .region_rects_for_splitter_resize(path, bounds, min_region_size)
    }

    pub fn resize_splitter_would_change(
        &self,
        tab_id: TabId,
        path: &SplitterPath,
        bounds: Rect,
        pointer_x: i32,
        pointer_y: i32,
        min_region_size: i32,
    ) -> DomainResult<bool> {
        self.tab(tab_id)?.resize_splitter_would_change(
            path,
            bounds,
            pointer_x,
            pointer_y,
            min_region_size,
        )
    }

    pub(crate) fn replace_tab_layout(
        &mut self,
        tab_id: TabId,
        layout: LayoutNode,
    ) -> DomainResult<()> {
        self.tab_mut(tab_id)?.layout = layout;
        Ok(())
    }

    pub(crate) fn restore_next_region_id(&mut self, next_region_id: u64) {
        self.next_region_id = next_region_id;
    }
}

// Placement lifecycle.
impl Workspace {
    pub fn remove_placement(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
    ) -> DomainResult<Placement> {
        let tab = self.tab_mut(tab_id)?;

        if !tab.layout().contains_region(region_id) {
            return Err(DomainError::RegionNotFound(region_id));
        }

        tab.remove_placement_by_region(region_id)
            .ok_or(DomainError::PlacementNotFound { tab_id, region_id })
    }

    pub(crate) fn remove_placement_for_rollback(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
    ) -> DomainResult<RemovedPlacement> {
        let tab = self.tab_mut(tab_id)?;

        if !tab.layout().contains_region(region_id) {
            return Err(DomainError::RegionNotFound(region_id));
        }

        tab.remove_placement_by_region_for_rollback(region_id)
            .ok_or(DomainError::PlacementNotFound { tab_id, region_id })
    }

    pub(crate) fn restore_removed_placement(
        &mut self,
        removed: RemovedPlacement,
    ) -> DomainResult<()> {
        self.ensure_window_not_placed(removed.hwnd())?;
        self.tab_mut(removed.tab_id())?
            .restore_removed_placement(removed)
    }

    pub fn move_placement(
        &mut self,
        tab_id: TabId,
        source_region_id: RegionId,
        target_region_id: RegionId,
    ) -> DomainResult<bool> {
        self.tab_mut(tab_id)?
            .move_placement(source_region_id, target_region_id)
    }

    pub fn ensure_can_move_placement(
        &self,
        tab_id: TabId,
        source_region_id: RegionId,
        target_region_id: RegionId,
    ) -> DomainResult<bool> {
        self.tab(tab_id)?
            .ensure_can_move_placement(source_region_id, target_region_id)
    }

    pub fn ensure_can_place(
        &self,
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
    ) -> DomainResult<()> {
        self.ensure_window_not_placed(hwnd)?;
        let tab = self.tab(tab_id)?;
        Self::ensure_region_can_accept_placement(tab, region_id)
    }

    pub fn place_window(
        &mut self,
        tab_id: TabId,
        region_id: RegionId,
        hwnd: WindowHandle,
        snapshot: WindowSnapshot,
    ) -> DomainResult<()> {
        self.ensure_can_place(tab_id, region_id, hwnd)?;
        let placement = Placement::new(tab_id, region_id, hwnd, snapshot)?;
        self.tab_mut(tab_id)?.add_placement(placement)
    }
}

// Layout queries.
impl Workspace {
    pub fn layout_for_tab(
        &self,
        tab_id: TabId,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Vec<RegionRect>> {
        self.tab(tab_id)?.layout_rects(bounds, min_region_size)
    }

    pub fn region_and_splitter_rects_for_tab(
        &self,
        tab_id: TabId,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<(Vec<RegionRect>, Vec<SplitterRect>)> {
        self.tab(tab_id)?
            .region_and_splitter_rects(bounds, tolerance, min_region_size)
    }

    pub fn region_and_splitter_rects_for_tab_into(
        &self,
        tab_id: TabId,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
        regions: &mut Vec<RegionRect>,
        splitters: &mut Vec<SplitterRect>,
    ) -> DomainResult<()> {
        self.tab(tab_id)?.region_and_splitter_rects_into(
            bounds,
            tolerance,
            min_region_size,
            regions,
            splitters,
        )
    }

    pub fn find_region_rect(
        &self,
        tab_id: TabId,
        region_id: RegionId,
        bounds: Rect,
        min_region_size: i32,
    ) -> DomainResult<Rect> {
        self.tab(tab_id)?
            .find_region_rect(region_id, bounds, min_region_size)
    }

    pub fn hit_test_region(
        &self,
        tab_id: TabId,
        bounds: Rect,
        x: i32,
        y: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<RegionId>> {
        self.tab(tab_id)?
            .hit_test_region(bounds, x, y, min_region_size)
    }

    pub fn splitter_rects(
        &self,
        tab_id: TabId,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<Vec<SplitterRect>> {
        self.tab(tab_id)?
            .splitter_rects(bounds, tolerance, min_region_size)
    }

    pub fn splitter_rects_for_tab_into(
        &self,
        tab_id: TabId,
        bounds: Rect,
        tolerance: i32,
        min_region_size: i32,
        splitters: &mut Vec<SplitterRect>,
    ) -> DomainResult<()> {
        self.tab(tab_id)?
            .splitter_rects_into(bounds, tolerance, min_region_size, splitters)
    }

    pub fn hit_test_splitter(
        &self,
        tab_id: TabId,
        bounds: Rect,
        x: i32,
        y: i32,
        tolerance: i32,
        min_region_size: i32,
    ) -> DomainResult<Option<SplitterRect>> {
        self.tab(tab_id)?
            .hit_test_splitter(bounds, x, y, tolerance, min_region_size)
    }

    pub fn placements_for_tab(&self, tab_id: TabId) -> DomainResult<&[Placement]> {
        Ok(self.tab(tab_id)?.placements())
    }

    pub fn take_all_placements(&mut self) -> Vec<Placement> {
        let mut placements = Vec::new();

        for tab in &mut self.tabs {
            placements.extend(tab.take_placements());
        }

        placements
    }
}

// Workspace internal helpers.
impl Workspace {
    fn reset_empty_id_counters(&mut self) {
        self.next_tab_id = EMPTY_WORKSPACE_NEXT_TAB_ID;
        self.next_region_id = EMPTY_WORKSPACE_NEXT_REGION_ID;
    }

    fn tab_index(&self, tab_id: TabId) -> DomainResult<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.id() == tab_id)
            .ok_or(DomainError::TabNotFound(tab_id))
    }

    fn active_tab_after_removed_index(&self, removed_index: usize) -> Option<TabId> {
        if self.tabs.is_empty() {
            None
        } else if removed_index < self.tabs.len() {
            Some(self.tabs[removed_index].id())
        } else {
            self.tabs.last().map(Tab::id)
        }
    }

    fn runtime_tabs_from_settings(tabs: Vec<TabSettings>) -> (Vec<Tab>, Option<u64>, Option<u64>) {
        let mut runtime_tabs = Vec::with_capacity(tabs.len());
        let mut max_tab_id = None;
        let mut max_region_id = None;

        for tab_settings in tabs {
            let tab_id = tab_settings.id().value();
            max_tab_id = Some(max_tab_id.map_or(tab_id, |max: u64| max.max(tab_id)));
            if let Some(region_id) = tab_settings.max_region_id() {
                max_region_id =
                    Some(max_region_id.map_or(region_id, |max: u64| max.max(region_id)));
            }
            runtime_tabs.push(tab_settings.into_runtime_tab_without_placements());
        }

        (runtime_tabs, max_tab_id, max_region_id)
    }

    fn validate_active_tab(active_tab_id: Option<TabId>, tabs: &[Tab]) -> DomainResult<()> {
        if let Some(active_tab_id) = active_tab_id
            && !tabs.iter().any(|tab| tab.id() == active_tab_id)
        {
            Err(DomainError::TabNotFound(active_tab_id))
        } else {
            Ok(())
        }
    }

    fn with_next_region_id_rollback<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> DomainResult<T>,
    ) -> DomainResult<T> {
        let previous_next_region_id = self.next_region_id;
        let result = operation(self);
        if result.is_err() {
            self.next_region_id = previous_next_region_id;
        }
        result
    }

    fn ensure_window_not_placed(&self, hwnd: WindowHandle) -> DomainResult<()> {
        if self
            .tabs
            .iter()
            .flat_map(Tab::placements)
            .any(|placement| placement.hwnd() == hwnd)
        {
            Err(DomainError::WindowAlreadyPlaced(hwnd))
        } else {
            Ok(())
        }
    }

    fn ensure_region_can_accept_placement(tab: &Tab, region_id: RegionId) -> DomainResult<()> {
        if !tab.layout().contains_region(region_id) {
            return Err(DomainError::RegionNotFound(region_id));
        }

        if tab
            .placements()
            .iter()
            .any(|placement| placement.region_id() == region_id)
        {
            return Err(DomainError::RegionAlreadyOccupied(region_id));
        }

        Ok(())
    }

    fn has_tab(&self, tab_id: TabId) -> bool {
        self.tab_index(tab_id).is_ok()
    }

    fn allocate_region_id(&mut self) -> DomainResult<RegionId> {
        let id = self.next_region_id;
        self.next_region_id = self
            .next_region_id
            .checked_add(1)
            .ok_or(DomainError::IdExhausted("region"))?;
        Ok(RegionId::new(id))
    }
}

impl RegionIdAllocator for Workspace {
    fn allocate_region(&mut self) -> DomainResult<RegionId> {
        self.allocate_region_id()
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

fn next_id_after_max(
    configured_next: u64,
    current_max: Option<u64>,
    empty_next: u64,
    scope: &'static str,
) -> DomainResult<u64> {
    let Some(current_max) = current_max else {
        return Ok(empty_next);
    };

    let minimum_next = current_max
        .checked_add(1)
        .ok_or(DomainError::IdExhausted(scope))?;
    Ok(configured_next.max(minimum_next))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot(hwnd: WindowHandle) -> DomainResult<WindowSnapshot> {
        Ok(WindowSnapshot::new(
            hwnd,
            Rect::new(10, 20, 300, 200)?,
            WindowDisplayState::Normal,
        ))
    }

    #[test]
    fn new_tab_starts_with_one_root_region() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;

        assert_eq!(tab_id, TabId::new(0));
        assert_eq!(workspace.active_tab_id(), Some(tab_id));
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].region_id(), RegionId::new(0));
        assert_eq!(regions[0].rect(), bounds);

        Ok(())
    }

    #[test]
    fn failed_add_tab_name_validation_does_not_consume_ids() -> DomainResult<()> {
        let mut workspace = Workspace::new();

        assert!(matches!(
            workspace.add_tab(""),
            Err(DomainError::EmptyTabName)
        ));
        assert!(matches!(
            workspace.add_tab("Work\0Hidden"),
            Err(DomainError::EmptyTabName)
        ));
        assert!(workspace.tabs().is_empty());
        assert_eq!(workspace.active_tab_id(), None);
        assert_eq!(workspace.next_tab_id(), 0);
        assert_eq!(workspace.next_region_id(), 0);

        let tab_id = workspace.add_tab("First")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;
        let settings = workspace.to_settings()?;

        assert_eq!(tab_id, TabId::new(0));
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].region_id(), RegionId::new(0));
        assert_eq!(workspace.next_tab_id(), 1);
        assert_eq!(workspace.next_region_id(), 1);
        assert_eq!(settings.next_tab_id(), 1);
        assert_eq!(settings.next_region_id(), 1);

        Ok(())
    }

    #[test]
    fn failed_add_tab_region_id_exhaustion_does_not_consume_tab_id() -> DomainResult<()> {
        let existing_tab = TabId::new(0);
        let settings = WorkspaceSettings::new(
            vec![TabSettings::new(
                existing_tab,
                "Existing",
                LayoutNode::single_region(RegionId::new(0)),
                Vec::new(),
            )?],
            Some(existing_tab),
            1,
            u64::MAX,
        )?;
        let (mut workspace, deferred_placements) = Workspace::from_settings_layout_only(settings)?;

        assert_eq!(deferred_placements, 0);
        assert_eq!(workspace.next_tab_id(), 1);
        assert_eq!(workspace.next_region_id(), u64::MAX);

        let result = workspace.add_tab("New");

        assert!(matches!(
            result,
            Err(DomainError::IdExhausted(scope)) if scope == "region"
        ));
        assert_eq!(workspace.tabs().len(), 1);
        assert_eq!(workspace.active_tab_id(), Some(existing_tab));
        assert_eq!(workspace.next_tab_id(), 1);
        assert_eq!(workspace.next_region_id(), u64::MAX);

        Ok(())
    }

    #[test]
    fn tab_names_reject_internal_nul() -> DomainResult<()> {
        let invalid = "Work\0Hidden";

        assert!(matches!(
            Tab::new(TabId::new(1), invalid, RegionId::new(1)),
            Err(DomainError::EmptyTabName)
        ));

        let mut tab = Tab::new(TabId::new(2), "Work", RegionId::new(2))?;
        assert!(matches!(
            tab.rename(invalid),
            Err(DomainError::EmptyTabName)
        ));
        assert_eq!(tab.name(), "Work");

        assert!(matches!(
            TabSettings::new(
                TabId::new(3),
                invalid,
                LayoutNode::single_region(RegionId::new(3)),
                Vec::new(),
            ),
            Err(DomainError::EmptyTabName)
        ));

        Ok(())
    }

    #[test]
    fn external_program_spec_records_arguments_and_trims_title() -> DomainResult<()> {
        let spec = ExternalProgramSpec::new_with_arguments(
            r"C:\Tools\editor.exe",
            ["--profile", "Work A"],
            Some(String::from("  Editor  ")),
        )?;

        assert_eq!(spec.executable_path(), r"C:\Tools\editor.exe");
        assert_eq!(
            spec.arguments()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["--profile", "Work A"]
        );
        assert_eq!(spec.title(), Some("Editor"));

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn external_program_spec_preserves_non_utf8_unix_executable_path() -> DomainResult<()> {
        use std::os::unix::ffi::OsStrExt;

        let executable_path = b"/tmp/editor-\xFF".to_vec();
        let spec = ExternalProgramSpec::new_with_unix_executable_path_bytes_and_arguments(
            executable_path.clone(),
            ["--profile"],
            Some(String::from("  Editor  ")),
        )?;

        assert_eq!(
            spec.executable_path_unix_bytes(),
            Some(executable_path.as_slice())
        );
        assert_eq!(
            spec.executable_path_os().as_bytes(),
            executable_path.as_slice()
        );
        assert_eq!(
            spec.arguments()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["--profile"]
        );
        assert_eq!(spec.title(), Some("Editor"));

        Ok(())
    }

    #[test]
    fn external_program_spec_rejects_nul_arguments() {
        assert!(matches!(
            ExternalProgramSpec::new_with_arguments(
                r"C:\Tools\editor.exe",
                ["--profile", "bad\0value"],
                None,
            ),
            Err(DomainError::InvalidProgramArgument)
        ));
    }

    #[test]
    fn tab_preset_program_specs_round_trip_in_leaf_order() -> DomainResult<()> {
        let mut preset = TabPreset::new(
            "Workbench",
            TabPresetNode::Split {
                direction: SplitDirection::Vertical,
                ratio: SplitRatio::new(0.5)?,
                first: Box::new(TabPresetNode::Split {
                    direction: SplitDirection::Horizontal,
                    ratio: SplitRatio::new(0.5)?,
                    first: Box::new(TabPresetNode::Region {
                        program: Some(ExternalProgramSpec::new(r"C:\Tools\a.exe", None)?),
                    }),
                    second: Box::new(TabPresetNode::Region {
                        program: Some(ExternalProgramSpec::new(r"C:\Tools\b.exe", None)?),
                    }),
                }),
                second: Box::new(TabPresetNode::Region {
                    program: Some(ExternalProgramSpec::new(r"C:\Tools\c.exe", None)?),
                }),
            },
        )?;

        assert_eq!(
            preset
                .program_specs()
                .iter()
                .map(|program| program.executable_path())
                .collect::<Vec<_>>(),
            vec![r"C:\Tools\a.exe", r"C:\Tools\b.exe", r"C:\Tools\c.exe"]
        );

        let replaced = preset.replace_program_specs([
            ExternalProgramSpec::new(r"C:\Edited\a.exe", None)?,
            ExternalProgramSpec::new(r"C:\Edited\b.exe", None)?,
            ExternalProgramSpec::new(r"C:\Edited\c.exe", None)?,
        ]);

        assert_eq!(replaced, 3);
        assert_eq!(
            preset
                .program_specs()
                .iter()
                .map(|program| program.executable_path())
                .collect::<Vec<_>>(),
            vec![r"C:\Edited\a.exe", r"C:\Edited\b.exe", r"C:\Edited\c.exe"]
        );

        Ok(())
    }

    #[test]
    fn tab_preset_rename_normalizes_and_rejects_empty_names() -> DomainResult<()> {
        let mut preset = TabPreset::new("Workbench", TabPresetNode::Region { program: None })?;

        preset.rename("  Review  ")?;
        assert_eq!(preset.name(), "Review");
        assert_eq!(preset.rename(" \t "), Err(DomainError::EmptyTabPresetName));
        assert_eq!(preset.name(), "Review");

        Ok(())
    }

    #[test]
    fn reorder_tab_before_moves_by_id_and_preserves_active_tab() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let first = workspace.add_tab("First")?;
        let second = workspace.add_tab("Second")?;
        let third = workspace.add_tab("Third")?;

        workspace.set_active_tab(second)?;
        let changed = workspace.reorder_tab_before(third, Some(first))?;
        let settings = workspace.to_settings()?;

        assert!(changed);
        assert_eq!(
            workspace.tabs().iter().map(Tab::id).collect::<Vec<_>>(),
            vec![third, first, second]
        );
        assert_eq!(workspace.active_tab_id(), Some(second));
        assert_eq!(settings.active_tab_id(), Some(second));
        assert_eq!(
            settings
                .tabs()
                .iter()
                .map(TabSettings::id)
                .collect::<Vec<_>>(),
            vec![third, first, second]
        );

        Ok(())
    }

    #[test]
    fn reorder_tab_before_reports_missing_ids_without_changing_order() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let first = workspace.add_tab("First")?;
        let second = workspace.add_tab("Second")?;
        let missing = TabId::new(999);

        let result = workspace.reorder_tab_before(first, Some(missing));

        assert!(matches!(
            result,
            Err(DomainError::TabNotFound(tab_id)) if tab_id == missing
        ));
        assert_eq!(
            workspace.tabs().iter().map(Tab::id).collect::<Vec<_>>(),
            vec![first, second]
        );

        Ok(())
    }

    #[test]
    fn deleting_active_tab_selects_next_tab_then_previous_tab() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let first = workspace.add_tab("First")?;
        let second = workspace.add_tab("Second")?;
        let third = workspace.add_tab("Third")?;

        workspace.set_active_tab(second)?;
        let middle_deletion = workspace.delete_tab(second)?;

        assert_eq!(middle_deletion.previous_active_tab(), Some(second));
        assert_eq!(middle_deletion.current_active_tab(), Some(third));
        assert_eq!(workspace.active_tab_id(), Some(third));
        assert_eq!(
            workspace.tabs().iter().map(Tab::id).collect::<Vec<_>>(),
            vec![first, third]
        );

        let tail_deletion = workspace.delete_tab(third)?;

        assert_eq!(tail_deletion.previous_active_tab(), Some(third));
        assert_eq!(tail_deletion.current_active_tab(), Some(first));
        assert_eq!(workspace.active_tab_id(), Some(first));
        assert_eq!(
            workspace.tabs().iter().map(Tab::id).collect::<Vec<_>>(),
            vec![first]
        );

        Ok(())
    }

    #[test]
    fn deleting_last_tab_clears_active_tab_and_preserves_empty_settings_policy() -> DomainResult<()>
    {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Only")?;

        let deletion = workspace.delete_tab(tab_id)?;
        let settings = workspace.to_settings()?;

        assert_eq!(deletion.previous_active_tab(), Some(tab_id));
        assert_eq!(deletion.current_active_tab(), None);
        assert_eq!(workspace.active_tab_id(), None);
        assert!(workspace.tabs().is_empty());
        assert_eq!(workspace.next_tab_id(), 0);
        assert_eq!(workspace.next_region_id(), 0);
        assert!(settings.tabs().is_empty());
        assert_eq!(settings.active_tab_id(), None);
        assert_eq!(settings.next_tab_id(), 0);
        assert_eq!(settings.next_region_id(), 0);

        let new_tab = workspace.add_tab("Tab 0")?;
        let regions = workspace.layout_for_tab(
            new_tab,
            Rect::new(0, 0, 800, 600)?,
            DEFAULT_MIN_REGION_SIZE,
        )?;

        assert_eq!(new_tab, TabId::new(0));
        assert_eq!(regions[0].region_id(), RegionId::new(0));

        Ok(())
    }

    #[test]
    fn restoring_last_tab_deletion_restores_id_counters() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Only")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let previous_next_tab_id = workspace.next_tab_id();
        let previous_next_region_id = workspace.next_region_id();

        let deletion = workspace.delete_tab(tab_id)?;

        assert_eq!(workspace.next_tab_id(), 0);
        assert_eq!(workspace.next_region_id(), 0);

        workspace.restore_deleted_tab(deletion);

        assert_eq!(workspace.next_tab_id(), previous_next_tab_id);
        assert_eq!(workspace.next_region_id(), previous_next_region_id);

        Ok(())
    }

    #[test]
    fn delete_tab_after_reorder_uses_tab_id_not_current_index() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let first = workspace.add_tab("First")?;
        let second = workspace.add_tab("Second")?;
        let third = workspace.add_tab("Third")?;

        workspace.set_active_tab(second)?;
        workspace.reorder_tab_before(third, Some(first))?;
        let deletion = workspace.delete_tab(first)?;

        assert_eq!(deletion.removed_tab().id(), first);
        assert_eq!(deletion.previous_active_tab(), Some(second));
        assert_eq!(deletion.current_active_tab(), Some(second));
        assert_eq!(workspace.active_tab_id(), Some(second));
        assert_eq!(
            workspace.tabs().iter().map(Tab::id).collect::<Vec<_>>(),
            vec![third, second]
        );

        Ok(())
    }

    #[test]
    fn split_region_uses_tree_and_keeps_existing_region_first() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let root_region = workspace.layout_for_tab(
            tab_id,
            Rect::new(0, 0, 800, 600)?,
            DEFAULT_MIN_REGION_SIZE,
        )?[0]
            .region_id();

        let new_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let regions = workspace.layout_for_tab(
            tab_id,
            Rect::new(0, 0, 1000, 600)?,
            DEFAULT_MIN_REGION_SIZE,
        )?;

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].region_id(), root_region);
        assert_eq!(regions[0].rect().width(), 500);
        assert_eq!(regions[1].region_id(), new_region);
        assert_eq!(regions[1].rect().width(), 500);

        Ok(())
    }

    #[test]
    fn missing_region_split_does_not_consume_region_id() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let next_region_id = workspace.next_region_id();
        let missing_region = RegionId::new(next_region_id + 100);

        let result = workspace.split_region(tab_id, missing_region, SplitDirection::Vertical);

        assert!(matches!(
            result,
            Err(DomainError::RegionNotFound(region)) if region == missing_region
        ));
        assert_eq!(workspace.next_region_id(), next_region_id);
        let new_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        assert_eq!(new_region.value(), next_region_id);

        Ok(())
    }

    #[test]
    fn split_region_keeps_existing_placement_on_first_child_region() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let hwnd = WindowHandle::new(77)?;

        workspace.place_window(tab_id, root_region, hwnd, sample_snapshot(hwnd)?)?;
        let new_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let placements = workspace.placements_for_tab(tab_id)?;

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), root_region);
        assert!(workspace.tab(tab_id)?.layout().contains_region(new_region));

        Ok(())
    }

    #[test]
    fn move_placement_updates_region_and_preserves_snapshot() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let source_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let target_region =
            workspace.split_region(tab_id, source_region, SplitDirection::Vertical)?;
        let hwnd = WindowHandle::new(91)?;
        let snapshot = sample_snapshot(hwnd)?;

        workspace.place_window(tab_id, source_region, hwnd, snapshot.clone())?;
        let moved = workspace.move_placement(tab_id, source_region, target_region)?;
        let placements = workspace.placements_for_tab(tab_id)?;

        assert!(moved);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].region_id(), target_region);
        assert_eq!(placements[0].hwnd(), hwnd);
        assert_eq!(placements[0].snapshot(), &snapshot);

        Ok(())
    }

    #[test]
    fn move_placement_rejects_occupied_target_region() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let source_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let target_region =
            workspace.split_region(tab_id, source_region, SplitDirection::Vertical)?;
        let source_hwnd = WindowHandle::new(92)?;
        let target_hwnd = WindowHandle::new(93)?;

        workspace.place_window(
            tab_id,
            source_region,
            source_hwnd,
            sample_snapshot(source_hwnd)?,
        )?;
        workspace.place_window(
            tab_id,
            target_region,
            target_hwnd,
            sample_snapshot(target_hwnd)?,
        )?;

        let result = workspace.move_placement(tab_id, source_region, target_region);
        let placements = workspace.placements_for_tab(tab_id)?;

        assert!(matches!(
            result,
            Err(DomainError::RegionAlreadyOccupied(region)) if region == target_region
        ));
        assert!(placements.iter().any(|placement| {
            placement.region_id() == source_region && placement.hwnd() == source_hwnd
        }));
        assert!(placements.iter().any(|placement| {
            placement.region_id() == target_region && placement.hwnd() == target_hwnd
        }));

        Ok(())
    }

    #[test]
    fn delete_region_removes_placement_and_collapses_to_sibling() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let deleted_region =
            workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let hwnd = WindowHandle::new(88)?;

        workspace.place_window(tab_id, deleted_region, hwnd, sample_snapshot(hwnd)?)?;
        let removed = workspace.delete_region(tab_id, deleted_region)?;
        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;

        assert_eq!(removed.as_ref().map(Placement::hwnd), Some(hwnd));
        assert_eq!(workspace.placements_for_tab(tab_id)?.len(), 0);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].region_id(), root_region);
        assert_eq!(regions[0].rect(), bounds);

        Ok(())
    }

    #[test]
    fn delete_region_preserves_sibling_subtree() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let right_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let bottom_right =
            workspace.split_region(tab_id, right_region, SplitDirection::Horizontal)?;

        workspace.delete_region(tab_id, root_region)?;
        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].region_id(), right_region);
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 800, 300)?);
        assert_eq!(regions[1].region_id(), bottom_right);
        assert_eq!(regions[1].rect(), Rect::new(0, 300, 800, 300)?);

        Ok(())
    }

    #[test]
    fn deleting_region_preserves_placements_in_collapsed_sibling_subtree() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let right_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let bottom_right =
            workspace.split_region(tab_id, right_region, SplitDirection::Horizontal)?;
        let right_hwnd = WindowHandle::new(201)?;
        let bottom_hwnd = WindowHandle::new(202)?;

        workspace.place_window(
            tab_id,
            right_region,
            right_hwnd,
            sample_snapshot(right_hwnd)?,
        )?;
        workspace.place_window(
            tab_id,
            bottom_right,
            bottom_hwnd,
            sample_snapshot(bottom_hwnd)?,
        )?;

        let removed = workspace.delete_region(tab_id, root_region)?;
        let placements = workspace.placements_for_tab(tab_id)?;
        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;

        assert!(removed.is_none());
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].region_id(), right_region);
        assert_eq!(placements[1].region_id(), bottom_right);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].region_id(), right_region);
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 800, 300)?);
        assert_eq!(regions[1].region_id(), bottom_right);
        assert_eq!(regions[1].rect(), Rect::new(0, 300, 800, 300)?);

        Ok(())
    }

    #[test]
    fn root_region_cannot_be_deleted() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let result = workspace.delete_region(tab_id, root_region);

        assert!(matches!(
            result,
            Err(DomainError::RootRegionCannotBeDeleted(region)) if region == root_region
        ));

        Ok(())
    }

    #[test]
    fn missing_region_delete_returns_domain_error() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let missing = RegionId::new(999);
        let result = workspace.delete_region(tab_id, missing);

        assert!(matches!(
            result,
            Err(DomainError::RegionNotFound(region)) if region == missing
        ));

        Ok(())
    }

    #[test]
    fn workspace_settings_preserve_options() -> DomainResult<()> {
        let options = WorkspaceOptions::new_with_language(true, UiLanguage::Korean);
        let settings = WorkspaceSettings::new_with_tab_presets_and_options(
            Vec::new(),
            None,
            1,
            1,
            Vec::new(),
            options,
        )?;

        assert_eq!(settings.options(), options);
        assert!(!WorkspaceOptions::default().dock_hidden_workspace_ui());
        assert_eq!(
            WorkspaceOptions::default().ui_language(),
            UiLanguage::English
        );

        Ok(())
    }

    #[test]
    fn nested_splitter_tree_calculates_region_rects_from_parent_bounds() -> DomainResult<()> {
        let left = RegionId::new(1);
        let right_top = RegionId::new(2);
        let right_bottom = RegionId::new(3);
        let layout = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.4)?,
            first: Box::new(LayoutNode::single_region(left)),
            second: Box::new(LayoutNode::Split {
                direction: SplitDirection::Horizontal,
                ratio: SplitRatio::new(0.25)?,
                first: Box::new(LayoutNode::single_region(right_top)),
                second: Box::new(LayoutNode::single_region(right_bottom)),
            }),
        };

        let regions =
            layout.region_rects(Rect::new(10, 20, 1000, 600)?, DEFAULT_MIN_REGION_SIZE)?;

        assert_eq!(
            regions,
            vec![
                RegionRect::new(left, Rect::new(10, 20, 400, 600)?),
                RegionRect::new(right_top, Rect::new(410, 20, 600, 150)?),
                RegionRect::new(right_bottom, Rect::new(410, 170, 600, 450)?),
            ]
        );

        Ok(())
    }

    #[test]
    fn layout_region_id_validation_rejects_duplicates_during_traversal() -> DomainResult<()> {
        let duplicate = RegionId::new(7);
        let layout = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.5)?,
            first: Box::new(LayoutNode::single_region(duplicate)),
            second: Box::new(LayoutNode::Split {
                direction: SplitDirection::Horizontal,
                ratio: SplitRatio::new(0.5)?,
                first: Box::new(LayoutNode::single_region(RegionId::new(11))),
                second: Box::new(LayoutNode::single_region(duplicate)),
            }),
        };

        let region_ids = layout.region_ids();
        let max_region_id = layout.max_region_id();

        assert!(matches!(
            region_ids,
            Err(DomainError::DuplicateRegion(region)) if region == duplicate
        ));
        assert!(matches!(
            max_region_id,
            Err(DomainError::DuplicateRegion(region)) if region == duplicate
        ));

        Ok(())
    }

    #[test]
    fn layout_max_region_id_uses_largest_region_value() -> DomainResult<()> {
        let layout = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.5)?,
            first: Box::new(LayoutNode::single_region(RegionId::new(2))),
            second: Box::new(LayoutNode::Split {
                direction: SplitDirection::Horizontal,
                ratio: SplitRatio::new(0.5)?,
                first: Box::new(LayoutNode::single_region(RegionId::new(42))),
                second: Box::new(LayoutNode::single_region(RegionId::new(7))),
            }),
        };

        assert_eq!(layout.max_region_id()?, Some(42));

        Ok(())
    }

    #[test]
    fn combined_rects_match_separate_layout_results() -> DomainResult<()> {
        let left = RegionId::new(1);
        let right_top = RegionId::new(2);
        let right_bottom = RegionId::new(3);
        let layout = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.4)?,
            first: Box::new(LayoutNode::single_region(left)),
            second: Box::new(LayoutNode::Split {
                direction: SplitDirection::Horizontal,
                ratio: SplitRatio::new(0.25)?,
                first: Box::new(LayoutNode::single_region(right_top)),
                second: Box::new(LayoutNode::single_region(right_bottom)),
            }),
        };
        let bounds = Rect::new(10, 20, 1000, 600)?;
        let tolerance = 5;

        let (regions, splitters) =
            layout.region_and_splitter_rects(bounds, tolerance, DEFAULT_MIN_REGION_SIZE)?;
        let mut reused_regions = Vec::with_capacity(regions.len());
        reused_regions.push(RegionRect::new(RegionId::new(99), Rect::new(0, 0, 1, 1)?));
        let region_capacity = reused_regions.capacity();
        let mut reused_splitters = Vec::with_capacity(splitters.len());
        reused_splitters.push(SplitterRect::new(
            SplitterPath::root(),
            SplitDirection::Vertical,
            Rect::new(0, 0, 1, 1)?,
        ));
        let splitter_capacity = reused_splitters.capacity();

        layout.region_and_splitter_rects_into(
            bounds,
            tolerance,
            DEFAULT_MIN_REGION_SIZE,
            &mut reused_regions,
            &mut reused_splitters,
        )?;

        assert_eq!(
            regions,
            layout.region_rects(bounds, DEFAULT_MIN_REGION_SIZE)?
        );
        assert_eq!(
            splitters,
            layout.splitter_rects(bounds, tolerance, DEFAULT_MIN_REGION_SIZE)?
        );
        assert_eq!(reused_regions, regions);
        assert_eq!(reused_splitters, splitters);
        assert_eq!(reused_regions.capacity(), region_capacity);
        assert_eq!(reused_splitters.capacity(), splitter_capacity);

        let mut splitter_only = Vec::with_capacity(splitters.len() + 1);
        splitter_only.push(SplitterRect::new(
            SplitterPath::root(),
            SplitDirection::Vertical,
            Rect::new(0, 0, 1, 1)?,
        ));
        let splitter_only_capacity = splitter_only.capacity();

        layout.splitter_rects_into(
            bounds,
            tolerance,
            DEFAULT_MIN_REGION_SIZE,
            &mut splitter_only,
        )?;

        assert_eq!(splitter_only, splitters);
        assert_eq!(splitter_only.capacity(), splitter_only_capacity);

        Ok(())
    }

    #[test]
    fn region_rect_calculation_clamps_stored_ratio_to_min_region_size() -> DomainResult<()> {
        let first = RegionId::new(1);
        let second = RegionId::new(2);
        let leading_clamped = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.01)?,
            first: Box::new(LayoutNode::single_region(first)),
            second: Box::new(LayoutNode::single_region(second)),
        };
        let trailing_clamped = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.99)?,
            first: Box::new(LayoutNode::single_region(first)),
            second: Box::new(LayoutNode::single_region(second)),
        };

        let leading_regions =
            leading_clamped.region_rects(Rect::new(0, 0, 200, 100)?, DEFAULT_MIN_REGION_SIZE)?;
        let trailing_regions =
            trailing_clamped.region_rects(Rect::new(0, 0, 200, 100)?, DEFAULT_MIN_REGION_SIZE)?;

        assert_eq!(leading_regions[0].rect(), Rect::new(0, 0, 64, 100)?);
        assert_eq!(leading_regions[1].rect(), Rect::new(64, 0, 136, 100)?);
        assert_eq!(trailing_regions[0].rect(), Rect::new(0, 0, 136, 100)?);
        assert_eq!(trailing_regions[1].rect(), Rect::new(136, 0, 64, 100)?);

        Ok(())
    }

    #[test]
    fn find_region_rect_matches_full_layout_result() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(10, 20, 1000, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let right_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        workspace.split_region(tab_id, right_region, SplitDirection::Horizontal)?;
        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;

        for region in regions {
            assert_eq!(
                workspace.find_region_rect(
                    tab_id,
                    region.region_id(),
                    bounds,
                    DEFAULT_MIN_REGION_SIZE,
                )?,
                region.rect()
            );
        }

        let missing_region = RegionId::new(999);
        assert!(matches!(
            workspace.find_region_rect(tab_id, missing_region, bounds, DEFAULT_MIN_REGION_SIZE),
            Err(DomainError::RegionNotFound(region_id)) if region_id == missing_region
        ));

        Ok(())
    }

    #[test]
    fn hit_test_returns_region_for_point_inside_layout() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(10, 20, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let right_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;

        assert_eq!(
            workspace.hit_test_region(tab_id, bounds, 100, 100, DEFAULT_MIN_REGION_SIZE)?,
            Some(root_region)
        );
        assert_eq!(
            workspace.hit_test_region(tab_id, bounds, 600, 100, DEFAULT_MIN_REGION_SIZE)?,
            Some(right_region)
        );
        assert_eq!(
            workspace.hit_test_region(tab_id, bounds, 900, 100, DEFAULT_MIN_REGION_SIZE)?,
            None
        );

        Ok(())
    }

    #[test]
    fn hit_test_uses_half_open_region_edges() -> DomainResult<()> {
        let left = RegionId::new(1);
        let right = RegionId::new(2);
        let layout = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.5)?,
            first: Box::new(LayoutNode::single_region(left)),
            second: Box::new(LayoutNode::single_region(right)),
        };
        let bounds = Rect::new(0, 0, 800, 600)?;

        assert_eq!(
            layout.hit_test(bounds, 399, 100, DEFAULT_MIN_REGION_SIZE)?,
            Some(left)
        );
        assert_eq!(
            layout.hit_test(bounds, 400, 100, DEFAULT_MIN_REGION_SIZE)?,
            Some(right)
        );
        assert_eq!(
            layout.hit_test(bounds, 800, 100, DEFAULT_MIN_REGION_SIZE)?,
            None
        );
        assert_eq!(
            layout.hit_test(bounds, 100, 600, DEFAULT_MIN_REGION_SIZE)?,
            None
        );

        Ok(())
    }

    #[test]
    fn splitter_hit_test_reports_nested_splitter_paths() -> DomainResult<()> {
        let left = RegionId::new(1);
        let right_top = RegionId::new(2);
        let right_bottom = RegionId::new(3);
        let layout = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.4)?,
            first: Box::new(LayoutNode::single_region(left)),
            second: Box::new(LayoutNode::Split {
                direction: SplitDirection::Horizontal,
                ratio: SplitRatio::new(0.25)?,
                first: Box::new(LayoutNode::single_region(right_top)),
                second: Box::new(LayoutNode::single_region(right_bottom)),
            }),
        };
        let bounds = Rect::new(10, 20, 1000, 600)?;
        let splitters = layout.splitter_rects(bounds, 5, DEFAULT_MIN_REGION_SIZE)?;
        let root_hit = layout
            .hit_test_splitter(bounds, 410, 50, 5, DEFAULT_MIN_REGION_SIZE)?
            .ok_or(DomainError::SplitterNotFound)?;
        let nested_hit = layout
            .hit_test_splitter(bounds, 500, 170, 5, DEFAULT_MIN_REGION_SIZE)?
            .ok_or(DomainError::SplitterNotFound)?;

        assert_eq!(splitters.len(), 2);
        assert_eq!(splitters[0].path().steps(), &[]);
        assert_eq!(splitters[0].direction(), SplitDirection::Vertical);
        assert_eq!(splitters[0].rect(), Rect::new(405, 20, 11, 600)?);
        assert_eq!(splitters[1].path().steps(), &[SplitterChild::Second]);
        assert_eq!(splitters[1].direction(), SplitDirection::Horizontal);
        assert_eq!(splitters[1].rect(), Rect::new(410, 165, 600, 11)?);
        assert_eq!(root_hit.path().steps(), &[]);
        assert_eq!(nested_hit.path().steps(), &[SplitterChild::Second]);

        Ok(())
    }

    #[test]
    fn split_ratio_calculates_from_first_child_size() -> DomainResult<()> {
        let ratio = SplitRatio::from_first_child_size(
            SplitDirection::Vertical,
            1000,
            400,
            DEFAULT_MIN_REGION_SIZE,
        )?;

        assert!((ratio.value() - 0.4).abs() < f64::EPSILON);
        assert!(matches!(
            SplitRatio::from_first_child_size(SplitDirection::Horizontal, 100, 50, 64),
            Err(DomainError::RegionTooSmall { .. })
        ));
        assert!(matches!(
            SplitRatio::from_first_child_size(SplitDirection::Horizontal, 1000, 20, 64),
            Err(DomainError::InvalidSplitPosition { .. })
        ));

        Ok(())
    }

    #[test]
    fn resizing_splitter_updates_ratio_and_layout() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 1000, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let right_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let splitter = workspace
            .hit_test_splitter(tab_id, bounds, 500, 200, 5, DEFAULT_MIN_REGION_SIZE)?
            .ok_or(DomainError::SplitterNotFound)?;

        workspace.resize_splitter(
            tab_id,
            splitter.path(),
            bounds,
            300,
            200,
            DEFAULT_MIN_REGION_SIZE,
        )?;
        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;

        assert_eq!(regions[0].region_id(), root_region);
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 300, 600)?);
        assert_eq!(regions[1].region_id(), right_region);
        assert_eq!(regions[1].rect(), Rect::new(300, 0, 700, 600)?);

        Ok(())
    }

    #[test]
    fn splitter_resize_would_change_reports_ratio_changes_without_mutating() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 1000, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let splitter = workspace
            .hit_test_splitter(tab_id, bounds, 500, 200, 5, DEFAULT_MIN_REGION_SIZE)?
            .ok_or(DomainError::SplitterNotFound)?;

        assert!(!workspace.resize_splitter_would_change(
            tab_id,
            splitter.path(),
            bounds,
            500,
            200,
            DEFAULT_MIN_REGION_SIZE,
        )?);
        assert!(workspace.resize_splitter_would_change(
            tab_id,
            splitter.path(),
            bounds,
            300,
            200,
            DEFAULT_MIN_REGION_SIZE,
        )?);

        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;
        assert_eq!(regions[0].rect(), Rect::new(0, 0, 500, 600)?);
        assert_eq!(regions[1].rect(), Rect::new(500, 0, 500, 600)?);

        Ok(())
    }

    #[test]
    fn splitter_resize_if_changed_returns_small_rollback_state() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 1000, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let right_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let splitter = workspace
            .hit_test_splitter(tab_id, bounds, 500, 200, 5, DEFAULT_MIN_REGION_SIZE)?
            .ok_or(DomainError::SplitterNotFound)?;

        assert!(
            workspace
                .resize_splitter_if_changed(
                    tab_id,
                    splitter.path(),
                    bounds,
                    500,
                    200,
                    DEFAULT_MIN_REGION_SIZE,
                )?
                .is_none()
        );

        let rollback = workspace
            .resize_splitter_if_changed(
                tab_id,
                splitter.path(),
                bounds,
                300,
                200,
                DEFAULT_MIN_REGION_SIZE,
            )?
            .ok_or(DomainError::SplitterNotFound)?;
        let regions = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;

        assert_eq!(regions[0].rect(), Rect::new(0, 0, 300, 600)?);
        assert_eq!(regions[1].rect(), Rect::new(300, 0, 700, 600)?);

        let rollback_regions = workspace.region_rects_before_splitter_resize(
            tab_id,
            splitter.path(),
            bounds,
            rollback,
            DEFAULT_MIN_REGION_SIZE,
        )?;

        assert_eq!(rollback_regions[0].region_id(), root_region);
        assert_eq!(rollback_regions[0].rect(), Rect::new(0, 0, 500, 600)?);
        assert_eq!(rollback_regions[1].region_id(), right_region);
        assert_eq!(rollback_regions[1].rect(), Rect::new(500, 0, 500, 600)?);

        workspace.rollback_splitter_resize(tab_id, splitter.path(), rollback)?;
        let restored = workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?;

        assert_eq!(restored[0].rect(), Rect::new(0, 0, 500, 600)?);
        assert_eq!(restored[1].rect(), Rect::new(500, 0, 500, 600)?);

        Ok(())
    }

    #[test]
    fn duplicate_window_cannot_be_placed_across_tabs() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let first_tab = workspace.add_tab("First")?;
        let second_tab = workspace.add_tab("Second")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let first_region =
            workspace.layout_for_tab(first_tab, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let second_region =
            workspace.layout_for_tab(second_tab, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let hwnd = WindowHandle::new(42)?;

        workspace.place_window(first_tab, first_region, hwnd, sample_snapshot(hwnd)?)?;
        let result =
            workspace.place_window(second_tab, second_region, hwnd, sample_snapshot(hwnd)?);

        assert!(matches!(result, Err(DomainError::WindowAlreadyPlaced(_))));

        Ok(())
    }

    #[test]
    fn workspace_settings_capture_tabs_layout_and_placements() -> DomainResult<()> {
        let mut workspace = Workspace::new();
        let tab_id = workspace.add_tab("Work")?;
        let bounds = Rect::new(0, 0, 800, 600)?;
        let root_region =
            workspace.layout_for_tab(tab_id, bounds, DEFAULT_MIN_REGION_SIZE)?[0].region_id();
        let right_region = workspace.split_region(tab_id, root_region, SplitDirection::Vertical)?;
        let hwnd = WindowHandle::new(101)?;

        workspace.place_window(tab_id, right_region, hwnd, sample_snapshot(hwnd)?)?;

        let settings = workspace.to_settings()?;
        let tab = &settings.tabs()[0];
        let placement = &tab.placements()[0];

        assert_eq!(settings.tabs().len(), 1);
        assert_eq!(settings.active_tab_id(), Some(tab_id));
        assert_eq!(settings.next_tab_id(), 1);
        assert_eq!(settings.next_region_id(), 2);
        assert_eq!(tab.id(), tab_id);
        assert_eq!(tab.name(), "Work");
        match tab.layout() {
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                assert_eq!(*direction, SplitDirection::Vertical);
                assert_eq!(ratio.value(), DEFAULT_SPLIT_RATIO.value());
                assert_eq!(**first, LayoutNode::single_region(root_region));
                assert_eq!(**second, LayoutNode::single_region(right_region));
            }
            LayoutNode::Region { .. } => return Err(DomainError::RegionNotFound(right_region)),
        }
        assert_eq!(placement.region_id(), right_region);
        assert_eq!(placement.hwnd(), hwnd);
        assert_eq!(placement.snapshot().hwnd(), hwnd);
        assert!(!placement.restore_policy().allows_auto_restore());

        Ok(())
    }

    #[test]
    fn workspace_settings_selects_first_tab_when_active_tab_is_missing() -> DomainResult<()> {
        let first_tab = TabId::new(5);
        let second_tab = TabId::new(6);
        let settings = WorkspaceSettings::new(
            vec![
                TabSettings::new(
                    first_tab,
                    "First",
                    LayoutNode::single_region(RegionId::new(50)),
                    Vec::new(),
                )?,
                TabSettings::new(
                    second_tab,
                    "Second",
                    LayoutNode::single_region(RegionId::new(60)),
                    Vec::new(),
                )?,
            ],
            None,
            7,
            61,
        )?;

        assert_eq!(settings.active_tab_id(), Some(first_tab));

        let (workspace, deferred_placements) = Workspace::from_settings_layout_only(settings)?;

        assert_eq!(deferred_placements, 0);
        assert_eq!(workspace.active_tab_id(), Some(first_tab));

        Ok(())
    }

    #[test]
    fn loading_settings_keeps_layout_but_defers_saved_placements() -> DomainResult<()> {
        let tab_id = TabId::new(7);
        let left = RegionId::new(11);
        let right = RegionId::new(12);
        let hwnd = WindowHandle::new(909)?;
        let layout = LayoutNode::Split {
            direction: SplitDirection::Vertical,
            ratio: SplitRatio::new(0.25)?,
            first: Box::new(LayoutNode::single_region(left)),
            second: Box::new(LayoutNode::single_region(right)),
        };
        let placement = SavedPlacement::new(
            right,
            hwnd,
            sample_snapshot(hwnd)?,
            SavedWindowRestorePolicy::SessionOnlyNoAutoRestore,
        )?;
        let settings = WorkspaceSettings::new(
            vec![TabSettings::new(tab_id, "Loaded", layout, vec![placement])?],
            Some(tab_id),
            1,
            1,
        )?;

        let (workspace, deferred_placements) = Workspace::from_settings_layout_only(settings)?;
        let regions = workspace.layout_for_tab(
            tab_id,
            Rect::new(0, 0, 800, 600)?,
            DEFAULT_MIN_REGION_SIZE,
        )?;

        assert_eq!(deferred_placements, 1);
        assert_eq!(workspace.active_tab_id(), Some(tab_id));
        assert_eq!(workspace.next_tab_id(), 8);
        assert_eq!(workspace.next_region_id(), 13);
        assert_eq!(workspace.placements_for_tab(tab_id)?.len(), 0);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].region_id(), left);
        assert_eq!(regions[1].region_id(), right);

        Ok(())
    }
}
