/*
Copyright 2023.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package v1alpha1

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

// EDIT THIS FILE!  THIS IS SCAFFOLDING FOR YOU TO OWN!
// NOTE: json tags are required.  Any new fields you add must have json tags for the fields to be serialized.

// LinkSpec defines the desired state of Link
type LinkSpec struct {
	Provider   Source `json:"provider"`
	Actor      Source `json:"actor"`
	ContractId string `json:"contractId"`
}

// LinkStatus defines the observed state of Link
type LinkStatus struct {
	ProviderKey string `json:"providerKey"`
	ActorKey    string `json:"actorKey"`

	Conditions []metav1.Condition `json:"conditions"`
}

type Source struct {
	Key  string `json:"key,omitempty"`
	Name string `json:"name,omitempty"`
}

// +kubebuilder:object:root=true
// +kubebuilder:resource:path=links,scope=Namespaced,categories=kasmcloud
// +kubebuilder:subresource:status
// +kubebuilder:printcolumn:name="ContractId",type="string",JSONPath=".spec.contractId"
// +kubebuilder:printcolumn:name="ActoryKey",type="string",JSONPath=".status.actorKey"
// +kubebuilder:printcolumn:name="ProviderKey",type="string",JSONPath=".status.providerKey"

// Link is the Schema for the links API
type Link struct {
	metav1.TypeMeta   `json:",inline"`
	metav1.ObjectMeta `json:"metadata,omitempty"`

	Spec   LinkSpec   `json:"spec,omitempty"`
	Status LinkStatus `json:"status,omitempty"`
}

//+kubebuilder:object:root=true

// LinkList contains a list of Link
type LinkList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitempty"`
	Items           []Link `json:"items"`
}

func init() {
	SchemeBuilder.Register(&Link{}, &LinkList{})
}
