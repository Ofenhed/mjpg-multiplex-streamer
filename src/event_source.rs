use inotify::{EventStream, Inotify};

enum EventSourceInner<'a, T> {
    Inotify(Inotify),
    EventStream(EventStream<&'a mut T>),
}

pub(crate) struct EventSource<'a, T: 'a> {
    source: Option<EventSourceInner<'a, T>>,
    buf: *mut T,
}

impl<'a, T> EventSource<'a, T>
where
    &'a mut T: AsRef<[u8]> + AsMut<[u8]>,
{
    pub(crate) fn as_inotify(&mut self) -> &mut Inotify {
        use EventSourceInner as Inner;
        {
            let source = &mut self.source;
            if let Some(Inner::Inotify(inotify)) = source {
                return unsafe { &mut *(inotify as *mut _) };
            } else {
                let Some(Inner::EventStream(event_stream)) = source.take() else {
                    unreachable!()
                };
                *source = Some(Inner::Inotify(event_stream.into_inotify()));
            }
        }
        self.as_inotify()
    }

    pub(crate) fn as_event_stream(&mut self) -> std::io::Result<&mut EventStream<&'a mut T>> {
        use EventSourceInner as Inner;
        {
            let source = &mut self.source;
            if let Some(Inner::EventStream(event_stream)) = source {
                return unsafe { Ok(&mut *(event_stream as *mut _)) };
            } else {
                let Some(Inner::Inotify(inotify)) = source.take() else {
                    unreachable!()
                };
                *source = Some(Inner::EventStream(unsafe {
                    inotify.into_event_stream(&mut *self.buf)?
                }));
            }
        }
        self.as_event_stream()
    }

    pub(crate) fn new(source: Inotify, buf: &'a mut T) -> Self {
        Self {
            source: Some(EventSourceInner::Inotify(source)),
            buf,
        }
    }
}
