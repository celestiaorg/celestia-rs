//! Descriptor indexing without rewriting unknown protobuf options.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use prost::Message;
use prost_types::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
};

use crate::Profile;
use crate::ReflectionError;
use crate::proto::{self, request::Operation, response::Result as Answer};

struct File {
    raw: Vec<u8>,
    descriptor: FileDescriptorProto,
}

/// Immutable descriptors and service registration for one listener.
pub struct Registry {
    profile: Profile,
    services: Vec<String>,
    files: BTreeMap<String, File>,
    symbols: BTreeMap<String, String>,
    messages: BTreeSet<String>,
    extensions: BTreeMap<(String, i32), String>,
    registered_extensions: BTreeMap<String, BTreeSet<i32>>,
    lookup_overrides: BTreeMap<String, Vec<File>>,
}

fn qualified(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.into()
    } else {
        format!("{prefix}.{name}")
    }
}

impl Registry {
    /// Indexes original serialized FileDescriptorProto messages. Unknown options
    /// remain in the original byte strings returned to reflection clients.
    pub fn new(
        profile: Profile,
        mut services: Vec<String>,
        descriptors: impl IntoIterator<Item = Vec<u8>>,
    ) -> Result<Self, prost::DecodeError> {
        services.sort();
        services.dedup();
        let mut registry = Self {
            profile,
            services,
            files: BTreeMap::new(),
            symbols: BTreeMap::new(),
            messages: BTreeSet::new(),
            extensions: BTreeMap::new(),
            registered_extensions: BTreeMap::new(),
            lookup_overrides: BTreeMap::new(),
        };
        for raw in descriptors {
            let descriptor = FileDescriptorProto::decode(raw.as_slice())?;
            registry
                .files
                .insert(descriptor.name().to_owned(), File { raw, descriptor });
        }
        // gogoreflection indexes symbols in registered service descriptors and
        // their dependencies. Its fallback resolves globally registered message
        // names, but does not expose arbitrary unregistered service symbols.
        let visible = if profile == Profile::App {
            let roots: Vec<_> = registry
                .files
                .iter()
                .filter_map(|(name, file)| {
                    let package = file.descriptor.package();
                    file.descriptor
                        .service
                        .iter()
                        .any(|service| {
                            registry
                                .services
                                .contains(&qualified(package, service.name()))
                        })
                        .then(|| name.clone())
                })
                .collect();
            registry.dependencies(roots)
        } else {
            registry.files.keys().cloned().collect()
        };
        let descriptors: Vec<_> = registry
            .files
            .iter()
            .map(|(name, f)| (name.clone(), f.descriptor.clone()))
            .collect();
        for (name, descriptor) in descriptors {
            let package = descriptor.package();
            let symbols = visible.contains(&name);
            for message in &descriptor.message_type {
                registry.index_message(&name, package, message, symbols);
            }
            if symbols {
                for en in &descriptor.enum_type {
                    registry.index_enum(&name, package, en);
                }
                for service in &descriptor.service {
                    let full = qualified(package, service.name());
                    registry.symbols.insert(full.clone(), name.clone());
                    for method in &service.method {
                        registry
                            .symbols
                            .insert(qualified(&full, method.name()), name.clone());
                    }
                }
            }
            for field in &descriptor.extension {
                registry.index_extension(&name, package, field, symbols);
            }
        }
        for (name, number) in registry.extensions.keys() {
            registry
                .registered_extensions
                .entry(name.clone())
                .or_default()
                .insert(*number);
        }
        Ok(registry)
    }

    /// Apply the runtime's actual extension registration table. In particular,
    /// SDK's gogo-first registry need not include every declared extension.
    #[must_use]
    pub fn with_registered_extensions(
        mut self,
        registered: impl IntoIterator<Item = (String, i32)>,
    ) -> Self {
        self.registered_extensions.clear();
        for (name, number) in registered {
            self.messages.insert(name.clone());
            self.registered_extensions
                .entry(name)
                .or_default()
                .insert(number);
        }
        self
    }

    pub(crate) fn with_lookup_overrides(
        mut self,
        entries: Vec<proto::LookupEntry>,
    ) -> Result<Self, prost::DecodeError> {
        for entry in entries {
            let mut files = Vec::new();
            for raw in entry.files {
                let descriptor = FileDescriptorProto::decode(raw.as_slice())?;
                files.push(File { raw, descriptor });
            }
            self.lookup_overrides.insert(entry.key, files);
        }
        Ok(self)
    }

    fn override_response(&self, key: &str, sent: &mut BTreeSet<String>) -> Option<Answer> {
        let files = self.lookup_overrides.get(key)?;
        let mut output = Vec::new();
        for (index, file) in files.iter().enumerate() {
            let name = file.descriptor.name().to_owned();
            if index == 0 || !sent.contains(&name) {
                sent.insert(name);
                output.push(file.raw.clone());
            }
        }
        Some(Answer::FileDescriptorResponse(
            proto::FileDescriptorResponse {
                file_descriptor_proto: output,
            },
        ))
    }

    fn dependencies(&self, roots: Vec<String>) -> BTreeSet<String> {
        let mut result = BTreeSet::new();
        let mut queue = VecDeque::from(roots);
        while let Some(name) = queue.pop_front() {
            if !result.insert(name.clone()) {
                continue;
            }
            if let Some(file) = self.files.get(&name) {
                queue.extend(file.descriptor.dependency.iter().cloned());
            }
        }
        result
    }

    fn index_message(
        &mut self,
        file: &str,
        prefix: &str,
        message: &DescriptorProto,
        symbols: bool,
    ) {
        let full = qualified(prefix, message.name());
        self.messages.insert(full.clone());
        // The global message-type fallback is available on the SDK listener.
        self.symbols.insert(full.clone(), file.into());
        for nested in &message.nested_type {
            self.index_message(file, &full, nested, symbols);
        }
        if symbols {
            for field in &message.field {
                self.symbols
                    .insert(qualified(&full, field.name()), file.into());
            }
            for oneof in &message.oneof_decl {
                self.symbols
                    .insert(qualified(&full, oneof.name()), file.into());
            }
            for en in &message.enum_type {
                self.index_enum(file, &full, en);
            }
        }
        for extension in &message.extension {
            self.index_extension(file, &full, extension, symbols);
        }
    }

    fn index_enum(&mut self, file: &str, prefix: &str, en: &EnumDescriptorProto) {
        let full = qualified(prefix, en.name());
        self.symbols.insert(full.clone(), file.into());
        for value in &en.value {
            // SDK's old reflection index nests values under the enum. Modern
            // protoregistry uses protobuf's enclosing-message/package scope.
            let scope = if self.profile == Profile::App {
                &full
            } else {
                prefix
            };
            self.symbols
                .insert(qualified(scope, value.name()), file.into());
        }
    }

    fn index_extension(
        &mut self,
        file: &str,
        prefix: &str,
        field: &FieldDescriptorProto,
        symbols: bool,
    ) {
        self.extensions.insert(
            (
                field.extendee().trim_start_matches('.').into(),
                field.number(),
            ),
            file.into(),
        );
        if symbols {
            self.symbols
                .insert(qualified(prefix, field.name()), file.into());
        }
    }

    fn descriptor_response(&self, root: &str, sent: &mut BTreeSet<String>) -> Answer {
        let mut files = Vec::new();
        let mut queue = VecDeque::from([root.to_owned()]);
        // Guard against invalid cyclic descriptor input; dependency traversal
        // order otherwise follows Go's breadth-first declaration order.
        let mut visited = BTreeSet::new();
        while let Some(name) = queue.pop_front() {
            if !visited.insert(name.clone()) {
                continue;
            }
            let Some(file) = self.files.get(&name) else {
                continue;
            };
            if files.is_empty() || !sent.contains(&name) {
                sent.insert(name);
                files.push(file.raw.clone());
            }
            queue.extend(file.descriptor.dependency.iter().cloned());
        }
        Answer::FileDescriptorResponse(proto::FileDescriptorResponse {
            file_descriptor_proto: files,
        })
    }

    fn not_found(message: String) -> Answer {
        Answer::ErrorResponse(proto::ErrorResponse {
            error_code: 5,
            error_message: message,
        })
    }

    fn missing_file(&self, name: &str) -> Answer {
        Self::not_found(if self.profile == Profile::App {
            format!("unknown file: {name}")
        } else {
            "proto: not found".into()
        })
    }

    fn extension_numbers(&self, name: &str) -> Answer {
        if self.messages.contains(name)
            || (self.profile == Profile::Core && self.symbols.contains_key(name))
        {
            let extension_number = self
                .registered_extensions
                .get(name)
                .map(|numbers| numbers.iter().copied().collect())
                .unwrap_or_default();
            Answer::AllExtensionNumbersResponse(proto::ExtensionNumberResponse {
                base_type_name: name.to_owned(),
                extension_number,
            })
        } else {
            Self::not_found(if self.profile == Profile::App {
                format!("unknown type: {name:?}")
            } else {
                "proto: not found".into()
            })
        }
    }

    /// Answers one request, keeping descriptor deduplication state only in the
    /// caller's stream. Lookup failures are in-band and leave the stream usable.
    pub fn answer(
        &self,
        request: proto::Request,
        sent: &mut BTreeSet<String>,
    ) -> Result<proto::Response, ReflectionError> {
        let result = match request.operation.as_ref() {
            Some(Operation::ListServices(_)) => {
                Answer::ListServicesResponse(proto::ListServiceResponse {
                    service: self
                        .services
                        .iter()
                        .cloned()
                        .map(|name| proto::ServiceResponse { name })
                        .collect(),
                })
            }
            Some(Operation::FileByFilename(name)) => {
                if self.files.contains_key(name) {
                    self.descriptor_response(name, sent)
                } else {
                    self.missing_file(name)
                }
            }
            Some(Operation::FileContainingSymbol(name)) => {
                if let Some(answer) = self.override_response(&format!("symbol:{name}"), sent) {
                    answer
                } else if let Some(file) = self.symbols.get(name) {
                    self.descriptor_response(file, sent)
                } else {
                    Self::not_found(if self.profile == Profile::App {
                        format!("unknown symbol: {name}")
                    } else {
                        "proto: not found".into()
                    })
                }
            }
            Some(Operation::FileContainingExtension(extension)) => {
                if let Some(answer) = self.override_response(
                    &format!(
                        "extension:{}:{}",
                        extension.containing_type, extension.extension_number
                    ),
                    sent,
                ) {
                    answer
                } else if let Some(file) = self.extensions.get(&(
                    extension.containing_type.clone(),
                    extension.extension_number,
                )) {
                    if self
                        .registered_extensions
                        .get(&extension.containing_type)
                        .is_some_and(|numbers| numbers.contains(&extension.extension_number))
                    {
                        self.descriptor_response(file, sent)
                    } else if self.profile == Profile::Core {
                        Self::not_found("proto: not found".into())
                    } else {
                        Self::not_found(format!(
                            "failed to find registered extension for extension number {}",
                            extension.extension_number
                        ))
                    }
                } else if self.profile == Profile::Core {
                    Self::not_found("proto: not found".into())
                } else if self.messages.contains(&extension.containing_type) {
                    Self::not_found(format!(
                        "failed to find registered extension for extension number {}",
                        extension.extension_number
                    ))
                } else {
                    Self::not_found(format!("unknown type: {:?}", extension.containing_type))
                }
            }
            Some(Operation::AllExtensionNumbersOfType(name)) => self.extension_numbers(name),
            None => {
                return Err(ReflectionError::invalid_argument(
                    "invalid MessageRequest: <nil>",
                ));
            }
        };
        Ok(proto::Response {
            valid_host: request.host.clone(),
            original_request: Some(request),
            result: Some(result),
        })
    }
}
