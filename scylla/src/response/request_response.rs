use std::collections::HashMap;

use bytes::Bytes;
use scylla_cql::frame::request::query::PagingStateResponse;
use scylla_cql::frame::response::{
    NonErrorResponseWithDeserializedMetadata, ResponseWithDeserializedMetadata,
};
use tracing::error;
use uuid::Uuid;

use crate::errors::RequestAttemptError;
use crate::frame::response::{self, result};
use crate::response::Coordinator;
use crate::response::query_result::QueryResult;

pub(crate) struct QueryResponse {
    pub(crate) response: ResponseWithDeserializedMetadata,
    pub(crate) tracing_id: Option<Uuid>,
    pub(crate) warnings: Vec<String>,
    // This is not exposed to user (yet?)
    pub(crate) custom_payload: Option<HashMap<String, Bytes>>,
}

// A QueryResponse in which response can not be Response::Error
pub(crate) struct NonErrorQueryResponse {
    pub(crate) response: NonErrorResponseWithDeserializedMetadata,
    pub(crate) tracing_id: Option<Uuid>,
    pub(crate) warnings: Vec<String>,
}

impl QueryResponse {
    pub(crate) fn into_non_error_query_response(
        self,
    ) -> Result<NonErrorQueryResponse, RequestAttemptError> {
        Ok(NonErrorQueryResponse {
            response: self.response.into_non_error_response()?,
            tracing_id: self.tracing_id,
            warnings: self.warnings,
        })
    }
}

impl NonErrorQueryResponse {
    pub(crate) fn as_set_keyspace(&self) -> Option<&result::SetKeyspace> {
        match &self.response {
            NonErrorResponseWithDeserializedMetadata::Result(
                result::ResultWithDeserializedMetadata::SetKeyspace(sk),
            ) => Some(sk),
            _ => None,
        }
    }

    pub(crate) fn as_schema_change(&self) -> Option<&result::SchemaChange> {
        match &self.response {
            NonErrorResponseWithDeserializedMetadata::Result(
                result::ResultWithDeserializedMetadata::SchemaChange(sc),
            ) => Some(sc),
            _ => None,
        }
    }

    fn into_query_result_and_paging_state_with_maybe_unknown_coordinator(
        self,
        request_coordinator: Option<Coordinator>,
    ) -> Result<(QueryResult, PagingStateResponse), RequestAttemptError> {
        let Self {
            response,
            tracing_id,
            warnings,
        } = self;
        let (raw_rows, paging_state_response) = match response {
            NonErrorResponseWithDeserializedMetadata::Result(
                result::ResultWithDeserializedMetadata::Rows((rs, paging_state_response)),
            ) => (Some(rs), paging_state_response),
            NonErrorResponseWithDeserializedMetadata::Result(_) => {
                (None, PagingStateResponse::NoMorePages)
            }
            _ => {
                return Err(RequestAttemptError::UnexpectedResponse(
                    response.to_response_kind(),
                ));
            }
        };

        Ok((
            match request_coordinator {
                Some(coordinator) => QueryResult::new(coordinator, raw_rows, tracing_id, warnings),
                None => QueryResult::new_with_unknown_coordinator(raw_rows, tracing_id, warnings),
            },
            paging_state_response,
        ))
    }

    /// Converts [NonErrorQueryResponse] into [QueryResult] and the associated [PagingStateResponse].
    pub(crate) fn into_query_result_and_paging_state(
        self,
        request_coordinator: Coordinator,
    ) -> Result<(QueryResult, PagingStateResponse), RequestAttemptError> {
        self.into_query_result_and_paging_state_with_maybe_unknown_coordinator(Some(
            request_coordinator,
        ))
    }

    fn into_query_result_with_maybe_unknown_coordinator(
        self,
        request_coordinator: Option<Coordinator>,
    ) -> Result<QueryResult, RequestAttemptError> {
        let (result, paging_state) = self
            .into_query_result_and_paging_state_with_maybe_unknown_coordinator(
                request_coordinator,
            )?;

        if !paging_state.finished() {
            error!(
                "Internal driver API misuse or a server bug: nonfinished paging state\
                would be discarded by `NonErrorQueryResponse::into_query_result`"
            );
            return Err(RequestAttemptError::NonfinishedPagingState);
        }

        Ok(result)
    }

    /// Converts [NonErrorQueryResponse] into [QueryResult]. Because it's intended to be used together with unpaged queries,
    /// it asserts that the associated [PagingStateResponse] is <finished> (says that there are no more pages left).
    pub(crate) fn into_query_result(
        self,
        request_coordinator: Coordinator,
    ) -> Result<QueryResult, RequestAttemptError> {
        self.into_query_result_with_maybe_unknown_coordinator(Some(request_coordinator))
    }

    /// The same as [Self::into_query_result()], but not omitting the [Coordinator].
    /// HACK: This is the way to create a [QueryResult] with `request_coordinator` set to [None].
    ///
    /// See [QueryResult::new_with_unknown_coordinator]
    pub(crate) fn into_query_result_with_unknown_coordinator(
        self,
    ) -> Result<QueryResult, RequestAttemptError> {
        self.into_query_result_with_maybe_unknown_coordinator(None)
    }
}

pub(crate) enum NonErrorStartupResponse {
    Ready,
    Authenticate(response::authenticate::Authenticate),
}

pub(crate) enum NonErrorAuthResponse {
    AuthChallenge(response::authenticate::AuthChallenge),
    AuthSuccess(response::authenticate::AuthSuccess),
}
