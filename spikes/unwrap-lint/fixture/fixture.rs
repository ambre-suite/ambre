// The approximations of the syn report, each one site the lint must judge.
// Compiled as the crate `ambre_fixture`, so every item is a workspace item.
#![allow(dead_code)]

pub mod names {
    pub struct Name(String);
    impl Name {
        pub fn as_str(&self) -> &str {
            &self.0
        }
    }
    pub struct Root<K>(String, core::marker::PhantomData<K>);
    impl<K> Root<K> {
        pub fn parse(text: &str) -> Option<Self> {
            Some(Self(text.to_owned(), core::marker::PhantomData))
        }
    }
}

mod a {
    pub struct Key(pub String);
    pub fn store(_: &str) {}
}
mod b {
    pub struct Key(pub u32);
}

pub mod sites {
    use crate::names::{Name, Root};

    // A refusal through an alias: syn places no alias of a generic type.
    pub type MarkRoot = Root<u8>;
    pub fn alias(name: &Name) -> bool {
        MarkRoot::parse(name.as_str()).is_some()
    }

    // A refusal on a receiver two globs would leave untyped for syn.
    use crate::a::*;
    #[allow(unused_imports)]
    use crate::b::*;
    pub fn glob(key: &crate::a::Key) {
        store(key.0.as_str());
    }

    // A foreign method on a receiver syn cannot type passes.
    pub fn foreign(name: &Name) -> usize {
        let text = name.as_str();
        text.chars().count()
    }

    // An unwrap kept in a binding, then handed on.
    pub fn binding(name: &Name) {
        let bare = name.as_str();
        crate::a::store(bare);
    }
}

// The census pass: one site per check that reads zero sites on the workspace.
pub mod census {
    // A newtype whose field is not private, read outside its module.
    pub mod inner {
        pub struct Open(pub u32);
    }
    pub fn read(open: &inner::Open) -> u32 {
        open.0
    }

    // A `Deref` to a primitive.
    pub struct Word(String);
    impl core::ops::Deref for Word {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }

    // A `bool` literal handed to a `pub` workspace `fn`.
    pub fn toggle(on: bool) -> bool {
        on
    }
    pub fn caller() -> bool {
        toggle(true)
    }
}
