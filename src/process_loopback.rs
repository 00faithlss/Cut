use std::{
    mem::{ManuallyDrop, size_of},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    time::{Duration, Instant},
};

use wasapi::{AudioClient, Direction};
use windows::Win32::{
    Media::Audio::{
        AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
        AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
        ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
        IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
        IAudioClient, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
        VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    },
    System::{
        Com::{
            BLOB,
            StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0},
        },
        Variant::VT_BLOB,
    },
};
use windows_core::{HRESULT, Interface, Ref, implement};

use crate::replay::Result;

const SLOW_OPEN_THRESHOLD: Duration = Duration::from_secs(2);

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationCallback {
    done: Sender<()>,
    _parameters: Box<AUDIOCLIENT_ACTIVATION_PARAMS>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationCallback_Impl {
    fn ActivateCompleted(
        &self,
        _: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows_core::Result<()> {
        let _ = self.done.send(());
        Ok(())
    }
}

pub(super) struct Activation {
    operation: IActivateAudioInterfaceAsyncOperation,
    done: Receiver<()>,
    warn_at: Option<Instant>,
}

pub(super) enum ActivationStatus {
    Pending,
    Delayed,
    Complete(Result<AudioClient>),
}

impl Activation {
    pub(super) fn start(pid: u32) -> Result<Self> {
        let (notify, done) = mpsc::channel();
        let mut parameters = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        });
        // Windows conserva el callback durante la operación. Los datos del BLOB
        // viajan con él si la fuente se desactiva antes de completar la apertura.
        let blob = BLOB {
            cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: std::ptr::from_mut(parameters.as_mut()).cast(),
        };
        let callback: IActivateAudioInterfaceCompletionHandler = ActivationCallback {
            done: notify,
            _parameters: parameters,
        }
        .into();
        let prop = ManuallyDrop::new(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BLOB,
                    Anonymous: PROPVARIANT_0_0_0 { blob },
                    ..Default::default()
                }),
            },
        });
        let operation = unsafe {
            ActivateAudioInterfaceAsync(
                VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                &IAudioClient::IID,
                Some(&*prop),
                &callback,
            )?
        };
        Ok(Self {
            operation,
            done,
            warn_at: Some(Instant::now() + SLOW_OPEN_THRESHOLD),
        })
    }

    pub(super) fn wait_for_probe(self) -> Result<AudioClient> {
        // Solo el hilo de diagnóstico espera aquí. El motor puede iniciar el
        // respaldo mientras Windows termina de abrir la captura por proceso.
        self.done.recv()?;
        self.client()
    }

    pub(super) fn poll(&mut self) -> ActivationStatus {
        match self.done.try_recv() {
            Ok(()) => ActivationStatus::Complete(self.client()),
            Err(TryRecvError::Empty) => {
                if self.warn_at.is_some_and(|at| Instant::now() >= at) {
                    self.warn_at = None;
                    ActivationStatus::Delayed
                } else {
                    ActivationStatus::Pending
                }
            }
            Err(TryRecvError::Disconnected) => ActivationStatus::Complete(Err(
                "Windows cerró la solicitud de audio de esta aplicación.".into(),
            )),
        }
    }

    fn client(&self) -> Result<AudioClient> {
        let mut result = HRESULT::default();
        let mut audio = None;
        unsafe { self.operation.GetActivateResult(&mut result, &mut audio)? };
        result.ok()?;
        let interface = audio.ok_or("Windows no devolvió una interfaz de captura.")?;
        Ok(AudioClient::from_iaudioclient(
            interface.cast()?,
            Direction::Render,
        ))
    }
}
