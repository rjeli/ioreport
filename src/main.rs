/*
extern CFMutableDictionaryRef IOReportCopyAllChannels(uint64_t, uint64_t);

extern int IOReportGetChannelCount(CFMutableDictionaryRef);

typedef struct IOReportSubscriptionRef *IOReportSubscriptionRef;
typedef CFDictionaryRef IOReportSampleRef;

extern IOReportSubscriptionRef
IOReportCreateSubscription(void *a, CFMutableDictionaryRef desiredChannels,
                           CFMutableDictionaryRef *subbedChannels,
                           uint64_t channel_id, CFTypeRef b);

CFMutableDictionaryRef channels;
    if (argc >= 2) {
        channels = IOReportCopyChannelsInGroup([NSString stringWithCString:argv[1] encoding:NSUTF8StringEncoding], 0x0, 0x0, 0x0, 0x0);
    } else {
        channels = IOReportCopyAllChannels(0x0, 0x0);
    }
*/

use std::{
    ffi,
    marker::{PhantomData, PhantomPinned},
    mem::MaybeUninit,
    ptr, thread,
    time::Duration,
};

use core_foundation::{
    array::CFArray,
    base::{CFTypeRef, TCFType},
    dictionary::{CFDictionary, CFDictionaryRef, CFMutableDictionary, CFMutableDictionaryRef},
    string::{CFString, CFStringRef},
};
use textplots::{Chart, Plot, Shape, TickDisplayBuilder};

type CFChannel = CFDictionary<CFString>;
type CFChannels = CFDictionary<CFString, CFArray<CFChannel>>;

#[repr(C)]
struct IOReportSubscription {
    _data: [u8; 0],
    _phantom: PhantomData<(*mut u8, PhantomPinned)>,
}

#[link(name = "IOReport")]
extern "C" {
    fn IOReportCopyAllChannels(a: u64, b: u64) -> CFDictionaryRef;
    fn IOReportCopyChannelsInGroup(
        group: CFStringRef,
        subgroup: CFStringRef,
        a: u64,
        b: u64,
        c: u64,
    ) -> CFDictionaryRef;

    fn IOReportChannelGetChannelName(ch: CFDictionaryRef) -> CFStringRef;

    /*
        extern IOReportSubscriptionRef
        IOReportCreateSubscription(void *a, CFMutableDictionaryRef desiredChannels,
                                   CFMutableDictionaryRef *subbedChannels,
                                   uint64_t channel_id, CFTypeRef b);

    extern NSDictionary *
    IOReportCreateSamples(IOReportSubscriptionRef iorsub,
                          CFMutableDictionaryRef subbedChannels, CFTypeRef a);
                                   */

    fn IOReportCreateSubscription(
        a: *const ffi::c_void,
        desired_channels: CFMutableDictionaryRef,
        subbed_channels: *mut CFMutableDictionaryRef,
        channel_id: u64,
        b: CFTypeRef,
    ) -> *const IOReportSubscription;

    fn IOReportCreateSamples(
        sub: *const IOReportSubscription,
        subbed_channels: CFMutableDictionaryRef,
        a: CFTypeRef,
    ) -> CFDictionaryRef;

    fn IOReportChannelGetFormat(channel: CFDictionaryRef) -> u8;
    fn IOReportSimpleGetIntegerValue(sample: CFDictionaryRef, a: *const ffi::c_void) -> u64;

    fn IOReportStateGetCount(sample: CFDictionaryRef) -> u32;
    fn IOReportStateGetResidency(sample: CFDictionaryRef, idx: u32) -> u64;
}

#[derive(Copy, Clone, Debug)]
struct WithSample;

#[derive(Debug)]
struct Channel<T>(CFChannel, PhantomData<T>);

impl Channel<()> {
    fn query_all() -> Vec<Self> {
        let channels = unsafe { CFChannels::wrap_under_create_rule(IOReportCopyAllChannels(0, 0)) };
        channels
            .get(CFString::from_static_string("IOReportChannels"))
            .into_iter()
            .map(|ch| Channel::retain(ch.as_concrete_TypeRef()))
            .collect()
    }
    fn query_group(group: &str, subgroup: Option<&str>) -> Vec<Self> {
        let group = CFString::new(group);
        let subgroup = subgroup.map(CFString::new);
        let channels = unsafe {
            CFChannels::wrap_under_create_rule(IOReportCopyChannelsInGroup(
                group.as_concrete_TypeRef(),
                subgroup
                    .map(|s| s.as_concrete_TypeRef())
                    .unwrap_or(ptr::null()),
                0,
                0,
                0,
            ))
        };
        channels
            .get(CFString::from_static_string("IOReportChannels"))
            .into_iter()
            .map(|ch| Channel::retain(ch.as_concrete_TypeRef()))
            .collect()
    }
}

impl<T> Clone for Channel<T> {
    fn clone(&self) -> Self {
        Self::retain(self.0.as_concrete_TypeRef())
    }
}

impl<T> Channel<T> {
    fn retain(d: CFDictionaryRef) -> Self {
        Self(unsafe { CFChannel::wrap_under_get_rule(d) }, PhantomData)
    }
    fn name(&self) -> String {
        unsafe {
            CFString::wrap_under_get_rule(IOReportChannelGetChannelName(
                self.0.as_concrete_TypeRef(),
            ))
        }
        .to_string()
    }
    /*
    fn get_str(&self, k: &NSString) -> Option<String> {
        Some(unsafe { Id::cast::<NSString>(self.0.get_retained(k)?) }.to_string())
    }
    fn group_name(&self) -> String {
        self.get_str(ns_string!("IOReportGroupName")).unwrap()
    }
    fn subgroup_name(&self) -> Option<String> {
        self.get_str(ns_string!("IOReportSubGroupName"))
    }
    */
}

#[derive(Debug, Clone)]
enum ChannelState {
    Invalid,
    Simple(u64),
    State(Vec<u64>),
}

impl Channel<WithSample> {
    fn get_state(&self) -> ChannelState {
        let format = unsafe { IOReportChannelGetFormat(self.0.as_concrete_TypeRef()) };
        match format {
            0 => ChannelState::Invalid,
            1 => ChannelState::Simple(unsafe {
                IOReportSimpleGetIntegerValue(self.0.as_concrete_TypeRef(), ptr::null())
            }),
            2 => {
                let count = unsafe { IOReportStateGetCount(self.0.as_concrete_TypeRef()) };
                ChannelState::State(
                    (0..count)
                        .map(|i| unsafe {
                            IOReportStateGetResidency(self.0.as_concrete_TypeRef(), i)
                        })
                        .collect(),
                )
            }
            _ => {
                println!("unhandled format: {format}");
                ChannelState::Invalid
            }
        }
    }
}

struct Subscription {
    sub: *const IOReportSubscription,
    subbed_channels: CFMutableDictionary,
}

impl Subscription {
    fn new(channels: &[Channel<()>]) -> Self {
        let channels: Vec<_> = channels.iter().map(|ch| ch.clone().0).collect();
        let channel_array = CFArray::from_CFTypes(&channels);

        let mut desired = CFMutableDictionary::new();
        desired.set(
            CFString::from_static_string("IOReportChannels"),
            channel_array.into_untyped(),
        );

        let mut subbed: MaybeUninit<CFMutableDictionaryRef> = MaybeUninit::uninit();
        let sub = unsafe {
            IOReportCreateSubscription(
                ptr::null(),
                desired.as_concrete_TypeRef(),
                subbed.as_mut_ptr(),
                0,
                ptr::null(),
            )
        };
        let subbed = unsafe { CFMutableDictionary::wrap_under_create_rule(subbed.assume_init()) };
        Self {
            sub,
            subbed_channels: subbed,
        }
    }
    fn get_samples(&self) -> Vec<Channel<WithSample>> {
        let samples: CFDictionary<CFString, CFArray<CFDictionary>> = unsafe {
            CFDictionary::wrap_under_create_rule(IOReportCreateSamples(
                self.sub,
                self.subbed_channels.as_concrete_TypeRef(),
                ptr::null(),
            ))
        };
        let sample_array = samples.get(CFString::from_static_string("IOReportChannels"));
        sample_array
            .into_iter()
            .map(|s| Channel::retain(s.as_concrete_TypeRef()))
            .collect()
    }
}

fn main() {
    println!("Hello, world!");

    // let all_channels = Channel::query_all();
    let channels = Channel::query_group("PMP", Some("DCS BW"));
    dbg!(channels.len());
    let channels: Vec<_> = channels
        .into_iter()
        .filter(|ch| ch.name() == "PACC0 RD+WR")
        .collect();

    for ch in &channels {
        dbg!(&ch);
        dbg!(ch.name());
    }

    let sub = Subscription::new(&channels);

    const INTERVAL_MS: u64 = 100;

    let mut old_state: Vec<u64> = vec![];

    loop {
        let ChannelState::State(state) = sub.get_samples()[0].get_state() else {
            panic!()
        };
        if !old_state.is_empty() {
            let delta: Vec<(f32, f32)> = (0..32)
                .map(|i| {
                    (
                        i as f32,
                        ((state[i] - old_state[i]) as f32) / (INTERVAL_MS as f32),
                    )
                })
                .collect();
            Chart::new_with_y_range(180, 60, 0.0, 32.0, 0.0, 10.0)
                .lineplot(&Shape::Steps(&delta))
                .display();
        }
        old_state = state;
        thread::sleep(Duration::from_millis(INTERVAL_MS));
    }
}
