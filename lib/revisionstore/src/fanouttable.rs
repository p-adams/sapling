/// Copyright Facebook, Inc. 2018
///
/// A FanoutTable trait for providing fast Node -> Bounds lookups to find bounds for bisecting.
/// It comes with two modes, small-mode keys off the first byte in the node, while large-mode keys
/// off the first two bytes in the node.
///
/// The serialization format is a big-endian serialized array of u32's, with one entry for each
/// possible 1 or 2 byte prefix. If nodes exist with that prefix, that fanout slot is set to the
/// offset of the earliest node with that prefix. If a fanout slot has no nodes, it's value is set
/// to the value of the last valid offset, or 0 if there is none.
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use failure::Fallible;
use std::io::{Cursor, Write};
use std::option::Option;
use types::node::Node;

const SMALL_FANOUT_FACTOR: u8 = 1;
const LARGE_FANOUT_FACTOR: u8 = 2;
const SMALL_FANOUT_LENGTH: usize = 256; // 2^8
const LARGE_FANOUT_LENGTH: usize = 65536; // 2^16
const SMALL_RAW_SIZE: usize = 1024; // SMALL_FANOUT_LENGTH * sizeof(u32)
const LARGE_RAW_SIZE: usize = 262144; // LARGE_FANOUT_LENGTH * sizeof(u32)

#[derive(Debug, Fail)]
#[fail(display = "Fanout Table Error: {:?}", _0)]
struct FanoutTableError(String);

fn get_fanout_index(table_size: usize, node: &Node) -> Fallible<u64> {
    let mut cursor = Cursor::new(node.as_ref());
    match table_size {
        SMALL_RAW_SIZE => Ok(cursor.read_u8()? as u64),
        LARGE_RAW_SIZE => Ok(cursor.read_u16::<BigEndian>()? as u64),
        _ => Err(
            FanoutTableError(format!("invalid fanout table size ({:?})", table_size).into()).into(),
        ),
    }
}

pub struct FanoutTable {}

impl FanoutTable {
    /// Returns the (start, end) search bounds indicated by the fanout table. If end is None, then
    /// search to the end of the index.
    pub fn get_bounds(table: &[u8], node: &Node) -> Fallible<(usize, Option<usize>)> {
        // Get the integer equivalent of the first few bytes of the node.
        let index = get_fanout_index(table.len(), node)?;

        // Read the start bound at the index location.
        let mut cur = Cursor::new(table);
        cur.set_position(index * 4);
        let start = cur.read_u32::<BigEndian>()? as usize;

        // Find the end bound by scanning forward for the first different entry.
        let mut end: Option<usize> = Option::None;
        while cur.position() < table.len() as u64 {
            let candidate = cur.read_u32::<BigEndian>()? as usize;
            if candidate != start {
                end = Option::Some(candidate as usize);
                break;
            }
        }

        Ok((start, end))
    }

    /// Writes a fanout table for the given list of sorted nodes.
    ///
    /// `fanout_factor` - Either '1' or '2', representing how many bytes should be used for the
    /// fanout.
    ///
    /// `node_iter` - The nodes that can be looked up in the fanout table. *MUST BE SORTED*
    ///
    /// `entry_size` - The fixed size of each node's index value, which is used to compute the
    /// offset in the index of that node's value.
    ///
    /// `locations` - A presized, mutable vector where the offset for each node index value will be
    /// written.
    pub fn write<'b, T: Write, I: Iterator<Item = &'b Node>>(
        writer: &mut T,
        fanout_factor: u8,
        node_iter: &mut I,
        entry_size: usize,
        mut locations: Option<&mut Vec<u32>>,
    ) -> Fallible<()> {
        let fanout_raw_size = match fanout_factor {
            SMALL_FANOUT_FACTOR => SMALL_RAW_SIZE,
            LARGE_FANOUT_FACTOR => LARGE_RAW_SIZE,
            _ => {
                return Err(FanoutTableError(
                    format!("invalid fanout factor ({:?})", fanout_factor).into(),
                )
                .into());
            }
        };
        let fanout_table_length = match fanout_factor {
            SMALL_FANOUT_FACTOR => SMALL_FANOUT_LENGTH,
            LARGE_FANOUT_FACTOR => LARGE_FANOUT_LENGTH,
            _ => {
                return Err(FanoutTableError(
                    format!("invalid fanout factor ({:?})", fanout_factor).into(),
                )
                .into());
            }
        };

        let mut fanout_table: Vec<Option<u32>> = vec![None; fanout_table_length];

        // Fill in the fanout table with the offset of the first entry for each prefix.
        let mut offset: u32 = 0;
        for (i, node) in node_iter.enumerate() {
            let fanout_key = get_fanout_index(fanout_raw_size, &node)?;
            if fanout_table[fanout_key as usize].is_none() {
                fanout_table[fanout_key as usize] = Some(offset);
            }
            if let Some(locations) = locations.as_mut() {
                locations[i] = offset;
            }
            offset += entry_size as u32;
        }

        // Serialize the fanout table. For fanout keys that have no value, use the previous valid
        // value.
        let mut last_offset = 0;
        for i in 0..fanout_table_length {
            let offset = match fanout_table[i] {
                Some(offset) => {
                    last_offset = offset;
                    offset
                }
                None => last_offset,
            };

            writer.write_u32::<BigEndian>(offset)?;
        }

        Ok(())
    }

    pub fn get_size(large: bool) -> usize {
        match large {
            true => LARGE_RAW_SIZE,
            false => SMALL_RAW_SIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    fn make_node(first: u8, second: u8, third: u8, fourth: u8) -> Node {
        let buf = [
            first, second, third, fourth, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        Node::from(&buf)
    }

    #[test]
    fn test_small_fanout() {
        let nodes: Vec<Node> = vec![
            make_node(0, 0, 0, 0),
            make_node(1, 0, 0, 0),
            make_node(1, 0, 0, 5),
            make_node(230, 5, 0, 0),
            make_node(230, 12, 0, 0),
        ];
        let mut locations = Vec::with_capacity(nodes.len());
        unsafe {
            locations.set_len(nodes.len());
        }
        let mut buf: Vec<u8> = vec![];
        FanoutTable::write(
            &mut buf,
            SMALL_FANOUT_FACTOR,
            &mut nodes.iter(),
            size_of::<u32>() as usize,
            Some(&mut locations),
        )
        .expect("fanout write");
        assert_eq!(SMALL_RAW_SIZE, buf.len());

        let table = buf.as_ref();
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[0]).expect("bounds0"),
            (0, Some(4))
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[1]).expect("bounds1"),
            (4, Some(12))
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[2]).expect("bounds2"),
            (4, Some(12))
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[3]).expect("bounds3"),
            (12, None)
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[4]).expect("bounds4"),
            (12, None)
        );
    }

    #[test]
    fn test_large_fanout() {
        let nodes: Vec<Node> = vec![
            make_node(0, 0, 0, 0),
            make_node(1, 0, 0, 0),
            make_node(1, 0, 0, 5),
            make_node(230, 5, 0, 0),
            make_node(230, 12, 0, 0),
        ];
        let mut locations = Vec::with_capacity(nodes.len());
        unsafe {
            locations.set_len(nodes.len());
        }
        let mut buf: Vec<u8> = vec![];
        FanoutTable::write(
            &mut buf,
            LARGE_FANOUT_FACTOR,
            &mut nodes.iter(),
            size_of::<u32>() as usize,
            Some(&mut locations),
        )
        .expect("fanout write");
        assert_eq!(LARGE_RAW_SIZE, buf.len());

        let table = buf.as_ref();
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[0]).expect("bounds0"),
            (0, Some(4))
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[1]).expect("bounds1"),
            (4, Some(12))
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[2]).expect("bounds2"),
            (4, Some(12))
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[3]).expect("bounds3"),
            (12, Some(16))
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[4]).expect("bounds4"),
            (16, None)
        );
    }

    #[test]
    fn test_empty() {
        let nodes: Vec<Node> = vec![];
        let mut locations = vec![];
        let mut buf: Vec<u8> = vec![];
        FanoutTable::write(
            &mut buf,
            SMALL_FANOUT_FACTOR,
            &mut nodes.iter(),
            size_of::<u32>() as usize,
            Some(&mut locations),
        )
        .expect("fanout write");
        assert_eq!(SMALL_RAW_SIZE, buf.len());

        let table = buf.as_ref();
        assert_eq!(
            FanoutTable::get_bounds(table, &make_node(0, 0, 0, 0)).expect("bounds1"),
            (0, None)
        );
    }

    /// Tests that lookups still work when all nodes in the pack start with the same fanout. To
    /// avoid bugs like in D8131020.
    #[test]
    fn test_same_prefix() {
        let nodes: Vec<Node> = vec![
            make_node(200, 0, 0, 0),
            make_node(200, 0, 0, 1),
            make_node(200, 0, 1, 5),
            make_node(200, 5, 0, 0),
            make_node(200, 12, 0, 0),
        ];
        let mut locations = Vec::with_capacity(nodes.len());
        unsafe {
            locations.set_len(nodes.len());
        }
        let mut buf: Vec<u8> = vec![];
        FanoutTable::write(
            &mut buf,
            SMALL_FANOUT_FACTOR,
            &mut nodes.iter(),
            size_of::<u32>() as usize,
            Some(&mut locations),
        )
        .expect("fanout write");
        assert_eq!(SMALL_RAW_SIZE, buf.len());

        let table = buf.as_ref();
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[0]).expect("bounds0"),
            (0, None)
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[1]).expect("bounds1"),
            (0, None)
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[2]).expect("bounds2"),
            (0, None)
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[3]).expect("bounds3"),
            (0, None)
        );
        assert_eq!(
            FanoutTable::get_bounds(table, &nodes[4]).expect("bounds4"),
            (0, None)
        );
    }

    quickcheck! {
        fn test_random_nodes(fanout: u8, nodes: Vec<Node>) -> bool {
            let mut nodes = nodes;
            nodes.sort();
            let fanout_factor = (fanout % 2) + 1;
            let mut locations = Vec::with_capacity(nodes.len());
            unsafe {
                locations.set_len(nodes.len());
            }
            let mut buf: Vec<u8> = vec![];
            let node_size = Node::len();
            FanoutTable::write(
                &mut buf,
                fanout_factor,
                &mut nodes.iter(),
                node_size,
                Some(&mut locations),
            ).expect("fanout write");

            // Simulate a data file that includes just the nodes
            let data_buf: Vec<u8> = nodes.iter().flat_map(|x| x.as_ref().iter()).map(|x| x.clone()).collect();

            // Validate the locations are correct
            for (i, node) in nodes.iter().enumerate() {
                let pos = locations[i] as usize;
                if &data_buf[pos..pos + node_size] != node.as_ref() {
                    return false;
                }
            }

            // Validate the returned bounds contain each node
            let table = buf.as_ref();
            for node in nodes.iter() {
                let (start, end) = FanoutTable::get_bounds(table, node).expect("bounds");
                let end = end.unwrap_or(data_buf.len());

                // Manually scan for the node in the data buffer bounds.
                let mut found = false;
                let mut cur = start;
                while start < end {
                    if &data_buf[cur..cur + node_size] == node.as_ref() {
                        found = true;
                        break;
                    }

                    cur += node_size;
                }
                if !found {
                    return false;
                }
            }

            true
        }
    }
}
