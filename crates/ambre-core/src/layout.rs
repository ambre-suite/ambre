//! The version byte in front of a stored layout.
//!
//! A register or a row that another build may read starts with one byte that
//! names its layout. The layouts a build knows form a closed set, so each
//! family of them is an enum, never a bare `u8`.

/// Defines the closed set of layouts one stored value has been written in.
///
/// `CURRENT` is the layout this build writes, and `byte` the byte that names a
/// layout. `from_byte` refuses a byte no known layout carries.
#[macro_export]
macro_rules! define_layout_version {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($(#[$vmeta:meta])* $variant:ident = $byte:literal),+ $(,)?
        }
        current = $current:ident;
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        $vis enum $name {
            $($(#[$vmeta])* $variant = $byte),+
        }

        impl $name {
            /// The layout this build writes.
            pub const CURRENT: Self = Self::$current;

            /// Bytes of the layout byte in front of what it lays out: one,
            /// the width `repr(u8)` gives it.
            pub const SIZE: $crate::ByteSize = $crate::ByteSize(1);

            /// The byte that names this layout in front of what it lays out.
            #[must_use]
            pub const fn byte(self) -> u8 {
                self as u8
            }

            /// The layout `byte` names, `None` for a byte no known layout
            /// carries.
            #[must_use]
            pub const fn from_byte(byte: u8) -> Option<Self> {
                match byte {
                    $($byte => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    define_layout_version! {
        /// A test layout family.
        pub enum Shape {
            /// The first layout.
            V1 = 1,
            /// The second layout.
            V2 = 2,
        }
        current = V2;
    }

    #[test]
    fn a_layout_reads_back_from_its_byte_and_an_unknown_byte_reads_as_none() {
        assert_eq!(Shape::CURRENT, Shape::V2);
        assert_eq!(Shape::from_byte(Shape::V1.byte()), Some(Shape::V1));
        assert_eq!(Shape::from_byte(Shape::CURRENT.byte()), Some(Shape::V2));
        assert_eq!(Shape::from_byte(0), None);
        assert_eq!(Shape::from_byte(3), None);
        assert_eq!(Shape::SIZE, crate::ByteSize(1));
    }
}
